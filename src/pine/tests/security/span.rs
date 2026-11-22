//! `request.security` for a timeframe the chart's bars do not fold into (a 45 minute candle on 30
//! minute bars), `calc_bars_count`, and a chart that opens inside a higher-timeframe candle.

use std::collections::HashMap;

use super::lookahead::{HOUR, hourly, hourly_bar, plot_values, session_with};
use crate::model::Kline;
use crate::pine::compiler::program::{BarEvent, BarPhase};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::runtime::session::RunError;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey, SeriesDepth};
use crate::pine::tests::bench::symbol;

/// Canned candles by key, for a provider that serves exactly what a test asks for.
#[derive(Clone, Default)]
struct CannedData(HashMap<SecurityKey, Vec<Kline>>);

impl CannedData {
    fn with(
        mut self,
        symbol: &str,
        interval: f64,
        depth: SeriesDepth,
        candles: Vec<Kline>,
    ) -> Self {
        let key = SecurityKey {
            symbol: symbol.to_owned(),
            interval,
            depth,
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
    let data =
        CannedData::default().with(&symbol().ticker_id, 2_700.0, SeriesDepth::Chart, candles);
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
    let data = CannedData::default().with("ETHUSDT", 14_400.0, SeriesDepth::Newest(2), candles);
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
    CannedData::default().with(
        &symbol().ticker_id,
        14_400.0,
        SeriesDepth::Chart,
        vec![first, second],
    )
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

const DAY: i64 = 24 * HOUR;
/// 2024-06-01 00:00 UTC.
const JUNE_FIRST: i64 = 1_717_200_000_000;

#[test]
fn calc_bars_count_counts_calendar_months_by_the_bars_not_by_elapsed_time() {
    // June, July and August as daily bars; a month is 30 days in Pine's seconds but 31 on the
    // calendar, so a window counted in elapsed time would end a day off.
    let chart: Vec<Kline> = (0..92)
        .map(|day| candle(JUNE_FIRST + day * DAY, DAY, 1.0, 1.0, 1.0))
        .collect();
    let read = |options: &str| {
        plotted(
            &format!(
                "request.security(syminfo.tickerid, \"1M\", ta.cum(1), lookahead = barmerge.lookahead_on{options})"
            ),
            &chart,
            CannedData::default(),
        )
    };
    let limited = read(", calc_bars_count = 2");
    assert_eq!(
        limited[29], None,
        "June is older than the newest two months"
    );
    assert_eq!(
        limited[30],
        Some(1.0),
        "July is the first month in the window"
    );
    assert_eq!(limited[91], Some(2.0), "July and August");
    assert_eq!(read("")[91], Some(3.0), "without the limit June counts too");
}

#[test]
fn a_calc_bars_count_no_series_can_hold_does_not_stall_the_run() {
    let chart = hourly(3);
    let source = "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"240\", close, calc_bars_count = 1000000000000), \"x\")\nplot(request.security(syminfo.tickerid, \"240\", close), \"y\")";
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut session = session_with(source, None);
        session.evaluate(&chart).expect("script runs");
        let _ = sender.send((plot_values(&session, "x"), plot_values(&session, "y")));
    });
    let (limited, plain) = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the run finishes");
    assert_eq!(limited, plain, "a count past the history is no limit");
}

/// Intrabars of the chart's own symbol whatever their depth, recording the depths asked for.
struct AnyDepth {
    candles: Vec<Kline>,
    asked: std::sync::Arc<std::sync::Mutex<Vec<SeriesDepth>>>,
}

impl SecurityDataProvider for AnyDepth {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        self.asked.lock().unwrap().push(key.depth);
        Some(self.candles.clone())
    }
}

#[test]
fn a_finer_timeframe_counts_candles_of_that_timeframe_the_forming_one_included() {
    // 4 hour chart bars at 00:00, 04:00 and 08:00, the last one still forming with a single hourly
    // candle (08:00) so far: TradingView, `"60"` with `calc_bars_count = 8` on a live 4h chart at
    // 08:xx, gave values to 3 chart bars, the 8 hours back.
    let chart: Vec<Kline> = (0..3)
        .map(|index| candle(index * 4 * HOUR, 4 * HOUR, 10.0, 10.0, 1.0))
        .collect();
    let hourly: Vec<Kline> = (0..=8)
        .map(|index| candle(index * HOUR, HOUR, 1.0, 1.0 + index as f64, 1.0))
        .collect();
    let read = |options: &str| {
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let provider = AnyDepth {
            candles: hourly.clone(),
            asked: asked.clone(),
        };
        let source = format!(
            "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"60\", close, lookahead = barmerge.lookahead_on{options}), \"x\")"
        );
        let mut session = session_with(&source, Some(Box::new(provider)));
        session.evaluate(&chart).expect("script runs");
        let depths = asked.lock().unwrap().clone();
        (plot_values(&session, "x"), depths)
    };
    // The first intrabar of each chart bar (`lookahead_on`): hours 0, 4 and 8 have closes 1, 5, 9.
    let (all, depth) = read("");
    assert_eq!(all, [Some(1.0), Some(5.0), Some(9.0)]);
    assert_eq!(depth, [SeriesDepth::Chart]);
    // The 8 newest hourly candles are 01:00-08:00: the first chart bar starts at 01:00.
    let (eight, depth) = read(", calc_bars_count = 8");
    assert_eq!(eight, [Some(2.0), Some(5.0), Some(9.0)]);
    assert_eq!(
        depth,
        [SeriesDepth::Newest(8)],
        "candles of the requested timeframe"
    );
    // Two: 07:00 and 08:00, so the first chart bar has none.
    assert_eq!(
        read(", calc_bars_count = 2").0,
        [None, Some(8.0), Some(9.0)]
    );
    assert_eq!(read(", calc_bars_count = 30").0, all);
}

#[test]
fn the_candle_forming_at_the_charts_end_is_the_newest_of_the_count() {
    // Sixty hourly bars: two whole days and half of the third, which is the one still forming.
    let chart = hourly(60);
    let read = |count: usize| {
        plotted(
            &format!(
                "request.security(syminfo.tickerid, \"1D\", ta.cum(1), lookahead = barmerge.lookahead_on, calc_bars_count = {count})"
            ),
            &chart,
            CannedData::default(),
        )
    };
    let one = read(1);
    assert_eq!(one.iter().flatten().count(), 12, "only today's 12 bars");
    assert_eq!(one[47], None);
    assert_eq!(one[48], Some(1.0));
    let three = read(3);
    assert_eq!(three.iter().flatten().count(), 60);
    assert_eq!(three[59], Some(3.0), "today and the two days before it");
    let two = read(2);
    assert_eq!(two[23], None);
    assert_eq!(two[24], Some(1.0));
    assert_eq!(two[59], Some(2.0));
}

/// The first plot of `source` after `history` loaded with a live bar to come and `live` ticked.
fn plotted_live(
    source: &str,
    provider: Option<Box<dyn SecurityDataProvider>>,
    history: &[Kline],
    live: Kline,
) -> Vec<Option<f64>> {
    let body = format!("//@version=6\nindicator(\"T\")\nplot({source}, \"x\")");
    let mut session = session_with(&body, provider);
    session
        .load_history(history, true, None)
        .expect("history runs");
    session
        .execute(
            BarEvent {
                candle: live,
                phase: BarPhase::RealtimeTick { is_new: true },
            },
            true,
            None,
        )
        .expect("the live bar runs");
    plot_values(&session, "x")
}

#[test]
fn a_live_bar_is_the_newest_chart_bar_of_calc_bars_count() {
    // A whole day of hourly bars, then a live bar that opens the next day: the window of the
    // newest day is the live bar's, not the history's last bar's.
    let history = hourly(24);
    let live = hourly_bar(24);
    let read = |count: usize| {
        plotted_live(
            &format!(
                "request.security(syminfo.tickerid, \"1D\", ta.cum(1), lookahead = barmerge.lookahead_on, calc_bars_count = {count})"
            ),
            None,
            &history,
            live.clone(),
        )
    };
    let one = read(1);
    assert_eq!(one.len(), 25);
    assert_eq!(one.iter().flatten().count(), 1, "only the live bar's day");
    assert_eq!(one[24], Some(1.0));
    let two = read(2);
    assert_eq!(two.iter().flatten().count(), 25, "yesterday and today");
    assert_eq!(two[24], Some(2.0));
}

#[test]
fn another_symbols_window_ends_at_the_live_bar_too() {
    let history = hourly(24);
    // Six whole 4h candles and the seventh, forming, which opens with the live bar at 00:00.
    let candles = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0]
        .iter()
        .enumerate()
        .map(|(index, close)| candle(index as i64 * 4 * HOUR, 4 * HOUR, *close, *close, 1.0))
        .collect();
    let data = CannedData::default().with("ETHUSDT", 14_400.0, SeriesDepth::Newest(1), candles);
    let values = plotted_live(
        "request.security(\"ETHUSDT\", \"240\", close, lookahead = barmerge.lookahead_on, calc_bars_count = 1)",
        Some(Box::new(data)),
        &history,
        hourly_bar(24),
    );
    assert_eq!(values.iter().flatten().count(), 1, "{values:?}");
    assert_eq!(values[24], Some(70.0));
}

#[test]
fn a_finer_request_with_a_live_bar_counts_back_from_it() {
    // 4 hour bars at 00:00 and 04:00, a live bar at 08:00 and hourly candles up to 08:00.
    let history: Vec<Kline> = (0..2)
        .map(|index| candle(index * 4 * HOUR, 4 * HOUR, 10.0, 10.0, 1.0))
        .collect();
    let hourly_candles: Vec<Kline> = (0..=8)
        .map(|index| candle(index * HOUR, HOUR, 1.0, 1.0 + index as f64, 1.0))
        .collect();
    let provider = AnyDepth {
        candles: hourly_candles,
        asked: Default::default(),
    };
    let values = plotted_live(
        "request.security(syminfo.tickerid, \"60\", close, lookahead = barmerge.lookahead_on, calc_bars_count = 8)",
        Some(Box::new(provider)),
        &history,
        candle(8 * HOUR, 4 * HOUR, 10.0, 10.0, 1.0),
    );
    // Hours 1-8 are the newest eight: the first chart bar starts at 01:00.
    assert_eq!(values, [Some(2.0), Some(5.0), Some(9.0)]);
}

const WEEK: i64 = 7 * DAY;

/// Daily candles of ETHUSDT, 28 of them from the epoch (four whole weeks), closing 100, 101, ...
fn eth_days(depth: SeriesDepth) -> CannedData {
    let days = (0..28)
        .map(|index| candle(index * DAY, DAY, 1.0, 100.0 + index as f64, 1.0))
        .collect();
    CannedData::default().with("ETHUSDT", 86_400.0, depth, days)
}

/// Four weekly chart bars from the epoch (a Thursday: the weeks are 7 days from there, which is
/// all the fold needs).
fn weekly_chart() -> Vec<Kline> {
    (0..4)
        .map(|index| candle(index * WEEK, WEEK, 10.0, 10.0, 1.0))
        .collect()
}

#[test]
fn a_window_that_cuts_a_chart_bar_gives_it_the_candles_inside_it() {
    let chart = weekly_chart();
    let read = |options: &str, depth: SeriesDepth| {
        plotted(
            &format!("request.security(\"ETHUSDT\", \"1D\", close{options})"),
            &chart,
            eth_days(depth),
        )
    };
    // No limit: each week reads the last daily close the week held (the last intrabar).
    assert_eq!(
        read("", SeriesDepth::Chart),
        [Some(106.0), Some(113.0), Some(120.0), Some(127.0)]
    );
    // The newest ten days are 18-27: the second week (days 14-20) is cut and keeps days 18-20.
    assert_eq!(
        read(", calc_bars_count = 10", SeriesDepth::Newest(10)),
        [None, None, Some(120.0), Some(127.0)]
    );
    // One day: only the last week, which holds it.
    assert_eq!(
        read(", calc_bars_count = 1", SeriesDepth::Newest(1)),
        [None, None, None, Some(127.0)]
    );
    // Eight days (20-27): days 20 and 21 are in different weeks.
    assert_eq!(
        read(", calc_bars_count = 8", SeriesDepth::Newest(8)),
        [None, None, Some(120.0), Some(127.0)]
    );
}

#[test]
fn a_cut_chart_bar_reads_the_first_candle_inside_the_window_with_lookahead_on() {
    let chart = weekly_chart();
    let read = |options: &str, depth: SeriesDepth| {
        plotted(
            &format!(
                "request.security(\"ETHUSDT\", \"1D\", close, lookahead = barmerge.lookahead_on{options})"
            ),
            &chart,
            eth_days(depth),
        )
    };
    assert_eq!(
        read("", SeriesDepth::Chart),
        [Some(100.0), Some(107.0), Some(114.0), Some(121.0)]
    );
    // Days 18-27: the cut week starts at its day 18 instead of 14.
    assert_eq!(
        read(", calc_bars_count = 10", SeriesDepth::Newest(10)),
        [None, None, Some(118.0), Some(121.0)]
    );
}

#[test]
fn a_foreign_candle_forming_on_the_live_bar_is_read_as_it_stands() {
    // Weekly chart: three whole weeks of history and a live bar for the fourth, with the weekly
    // candles of another symbol, the fourth still forming (its close so far 40).
    let history: Vec<Kline> = (0..3)
        .map(|index| candle(index * WEEK, WEEK, 10.0, 10.0, 1.0))
        .collect();
    let weeks = [10.0, 20.0, 30.0, 40.0]
        .iter()
        .enumerate()
        .map(|(index, close)| candle(index as i64 * WEEK, WEEK, 1.0, *close, 1.0))
        .collect::<Vec<_>>();
    let values = |options: &str, depth: SeriesDepth| {
        plotted_live(
            &format!("request.security(\"ETHUSDT\", \"1W\", close{options})"),
            Some(Box::new(CannedData::default().with(
                "ETHUSDT",
                604_800.0,
                depth,
                weeks.clone(),
            ))),
            &history,
            candle(3 * WEEK, WEEK, 10.0, 10.0, 1.0),
        )
    };
    let plain = values("", SeriesDepth::Chart);
    assert_eq!(
        plain[3],
        Some(40.0),
        "the live bar reads the developing candle"
    );
    let counted = values(", calc_bars_count = 3", SeriesDepth::Newest(3));
    assert_eq!(counted[3], Some(40.0));
    assert_eq!(
        counted[0], None,
        "the first week is older than the newest three"
    );
}
