//! `request.security` with `lookahead = barmerge.lookahead_on`, measured against TradingView (probe
//! P04, 16,497 BINANCE:BTCUSDT 1h bars, 0 mismatches there): on a historical chart bar the request
//! returns the higher-timeframe bar as it ended, from the period's first chart bar on, because the
//! chart's later bars exist already. The `expr[1]` idiom, `gaps_on`, a finer timeframe and a live
//! bar are covered beside it.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::program::{BarEvent, BarPhase};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::runtime::session::Session;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey, SeriesDepth};
use crate::pine::tests::bench::{limits, run_script, symbol};

pub(super) const HOUR: i64 = 3_600_000;
const FOUR_HOURS: usize = 4;

/// An hourly bar whose five values all differ from its neighbours', so a bucket folded from the
/// wrong bars changes at least one of them.
pub(super) fn hourly_bar(hour: i64) -> Kline {
    let open = 100.0 + hour as f64;
    Kline {
        open_time: hour * HOUR,
        close_time: (hour + 1) * HOUR - 1,
        open,
        high: open + 5.0 + (hour % 3) as f64,
        low: open - 5.0 - (hour % 2) as f64,
        close: open + ((hour * 7) % 5) as f64 - 2.0,
        volume: 1.0 + hour as f64,
    }
}

pub(super) fn hourly(count: i64) -> Vec<Kline> {
    (0..count).map(hourly_bar).collect()
}

/// The bars of `chart` in the 4h bucket `bucket`, folded the way a 4h feed would build the candle.
fn folded(chart: &[Kline], bucket: usize) -> Kline {
    let members = &chart[bucket * FOUR_HOURS..((bucket + 1) * FOUR_HOURS).min(chart.len())];
    let mut candle = members[0].clone();
    for bar in &members[1..] {
        candle.absorb(bar);
    }
    candle
}

pub(super) fn plot_values(session: &Session, title: &str) -> Vec<Option<f64>> {
    session
        .output()
        .plots
        .iter()
        .find(|plot| plot.title.as_deref() == Some(title))
        .unwrap_or_else(|| panic!("no plot {title}"))
        .values
        .clone()
}

/// The plots of `plots` (title and expression), each over `chart`.
fn run(plots: &[(&str, &str)], chart: &[Kline]) -> Session {
    let body: String = plots
        .iter()
        .map(|(title, expression)| format!("plot({expression}, \"{title}\")\n"))
        .collect();
    let source = format!("//@version=6\nindicator(\"Lookahead\")\n{body}");
    run_script(&source, chart, false, |_| {})
}

fn request(expression: &str, options: &str) -> String {
    format!("request.security(syminfo.tickerid, \"240\", {expression}{options})")
}

const AHEAD: &str = ", lookahead = barmerge.lookahead_on";

#[test]
fn lookahead_on_returns_the_completed_bar_from_the_periods_start() {
    let chart = hourly(12);
    let session = run(
        &[
            ("open", &request("open", AHEAD)),
            ("high", &request("high", AHEAD)),
            ("low", &request("low", AHEAD)),
            ("close", &request("close", AHEAD)),
            ("volume", &request("volume", AHEAD)),
        ],
        &chart,
    );
    for (title, read) in [
        ("open", (|c: &Kline| c.open) as fn(&Kline) -> f64),
        ("high", |c| c.high),
        ("low", |c| c.low),
        ("close", |c| c.close),
        ("volume", |c| c.volume),
    ] {
        let values = plot_values(&session, title);
        for (hour, value) in values.iter().enumerate() {
            let expected = read(&folded(&chart, hour / FOUR_HOURS));
            assert_eq!(*value, Some(expected), "{title} on hour {hour}");
        }
    }
}

#[test]
fn an_indicator_over_the_period_reads_its_completed_bar_too() {
    let chart = hourly(24);
    let ema = request("ta.ema(close, 3)", AHEAD);
    let late = request("ta.ema(close, 3)", "");
    let session = run(&[("ahead", &ema), ("late", &late)], &chart);
    let ahead = plot_values(&session, "ahead");
    let late = plot_values(&session, "late");
    // At each period's last bar the default request has just completed the bar `lookahead_on`
    // returned from its first.
    for bucket in 0..6 {
        let first = bucket * FOUR_HOURS;
        let last = first + FOUR_HOURS - 1;
        assert_eq!(ahead[first], late[last], "bucket {bucket}");
        assert_eq!(ahead[first], ahead[last], "bucket {bucket} is steady");
    }
}

#[test]
fn the_offset_idiom_is_the_previous_completed_bar_from_the_periods_start() {
    let chart = hourly(12);
    let session = run(&[("previous", &request("close[1]", AHEAD))], &chart);
    let values = plot_values(&session, "previous");
    for (hour, value) in values.iter().enumerate() {
        let bucket = hour / FOUR_HOURS;
        let expected = (bucket > 0).then(|| folded(&chart, bucket - 1).close);
        assert_eq!(*value, expected, "hour {hour}");
    }
}

#[test]
fn without_lookahead_a_period_is_known_only_after_its_last_bar() {
    let chart = hourly(12);
    let session = run(&[("late", &request("close", ""))], &chart);
    let values = plot_values(&session, "late");
    for (hour, value) in values.iter().enumerate() {
        let bucket = hour / FOUR_HOURS;
        let completed = if hour % FOUR_HOURS == FOUR_HOURS - 1 {
            Some(bucket)
        } else {
            bucket.checked_sub(1)
        };
        let expected = completed.map(|bucket| folded(&chart, bucket).close);
        assert_eq!(*value, expected, "hour {hour}");
    }
}

#[test]
fn gaps_on_with_lookahead_on_keeps_the_value_on_the_periods_first_bar_only() {
    let chart = hourly(12);
    let session = run(
        &[(
            "gapped",
            &request(
                "close",
                ", gaps = barmerge.gaps_on, lookahead = barmerge.lookahead_on",
            ),
        )],
        &chart,
    );
    let values = plot_values(&session, "gapped");
    for (hour, value) in values.iter().enumerate() {
        let expected = (hour % FOUR_HOURS == 0).then(|| folded(&chart, hour / FOUR_HOURS).close);
        assert_eq!(*value, expected, "hour {hour}");
    }
}

#[test]
fn a_call_under_a_condition_sees_what_one_that_always_runs_sees() {
    let chart = hourly(24);
    let call = request("ta.ema(close, 3)", AHEAD);
    let session = run(
        &[
            ("always", &call),
            ("gated", &format!("bar_index % 3 == 1 ? {call} : na")),
        ],
        &chart,
    );
    let always = plot_values(&session, "always");
    let gated = plot_values(&session, "gated");
    assert!(
        gated.iter().flatten().count() >= 5,
        "the call ran on several bars"
    );
    for (hour, (a, g)) in always.iter().zip(&gated).enumerate() {
        if g.is_some() {
            assert_eq!(a, g, "hour {hour}");
        }
    }
}

/// A run that gets only the first `known` bars of the chart sees the period end where they end:
/// the bars a replay has not reached yet are not in the session.
#[test]
fn bars_the_session_was_not_given_are_not_looked_ahead_to() {
    let chart = hourly(12);
    let call = request("close", AHEAD);
    let whole = plot_values(&run(&[("close", &call)], &chart), "close");
    let cut = plot_values(&run(&[("close", &call)], &chart[..6]), "close");
    assert_eq!(
        whole[5],
        Some(chart[7].close),
        "the whole chart knows hour 7"
    );
    assert_eq!(
        cut[5],
        Some(chart[5].close),
        "six bars end the period at hour 5"
    );
    assert_eq!(whole[..4], cut[..4], "a period that ended is the same");
}

#[test]
fn a_request_inside_a_request_looks_ahead_on_the_outer_series() {
    let chart = hourly(48);
    let nested = format!(
        "request.security(syminfo.tickerid, \"240\", request.security(syminfo.tickerid, \"1D\", close{AHEAD}){AHEAD})"
    );
    let session = run(&[("nested", &nested)], &chart);
    let values = plot_values(&session, "nested");
    for (hour, value) in values.iter().enumerate() {
        let day_close = chart[(hour / 24) * 24 + 23].close;
        assert_eq!(*value, Some(day_close), "hour {hour}");
    }
}

/// Four bars per period. The first of them closes below its open and the period as a whole closes
/// above it, so a strategy that compares the period's open and close on its first bar enters only
/// when it sees the period as it ended.
fn rising_periods(periods: i64) -> Vec<Kline> {
    (0..periods * FOUR_HOURS as i64)
        .map(|hour| {
            let step = hour % FOUR_HOURS as i64;
            let (open, close) = if step == 0 {
                (100.0, 99.0)
            } else {
                (100.0 + step as f64, 101.0 + step as f64)
            };
            Kline {
                open_time: hour * HOUR,
                close_time: (hour + 1) * HOUR - 1,
                open,
                high: open.max(close) + 1.0,
                low: open.min(close) - 1.0,
                close,
                volume: 1.0,
            }
        })
        .collect()
}

#[test]
fn a_strategy_that_reads_the_completed_period_trades_on_it() {
    let source = format!(
        "//@version=6
strategy(\"Period\", overlay = true)
periodOpen = {open}
periodClose = {close}
if bar_index % 4 == 0 and periodClose > periodOpen
    strategy.entry(\"L\", strategy.long)
if bar_index % 4 == 3
    strategy.close(\"L\")
",
        open = request("open", AHEAD),
        close = request("close", AHEAD),
    );
    let chart = rising_periods(5);
    let session = run_script(&source, &chart, false, |_| {});
    let report = session.output().strategy.expect("a strategy report");
    // One entry at the start of each of the five periods: four closed by the next period's first
    // bar, the last still open.
    assert_eq!(report.trades.len() + report.open_trades.len(), 5);
}

#[test]
fn a_finer_timeframe_with_lookahead_on_reads_the_first_intrabar() {
    let intrabar = |open_time: i64, close: f64| Kline {
        open_time,
        close_time: open_time + 14_999,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
    };
    let provider = Intrabars {
        key: SecurityKey {
            symbol: symbol().ticker_id,
            interval: 15.0,
            depth: SeriesDepth::Chart,
        },
        candles: vec![
            intrabar(60_000, 1.0),
            intrabar(75_000, 2.0),
            intrabar(90_000, 3.0),
            intrabar(105_000, 4.0),
        ],
    };
    let chart = [
        Kline {
            open_time: 0,
            close_time: 59_999,
            open: 10.0,
            high: 10.0,
            low: 10.0,
            close: 10.0,
            volume: 1.0,
        },
        Kline {
            open_time: 60_000,
            close_time: 119_999,
            open: 11.0,
            high: 11.0,
            low: 11.0,
            close: 11.0,
            volume: 1.0,
        },
    ];
    let read = |options: &str| {
        let source = format!(
            "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"15S\", close{options}), \"x\")"
        );
        let mut session = session_with(&source, Some(Box::new(provider.clone())));
        session.evaluate(&chart).expect("script runs");
        plot_values(&session, "x")
    };
    assert_eq!(read(""), [None, Some(4.0)], "the last intrabar");
    assert_eq!(read(AHEAD), [None, Some(1.0)], "the first intrabar");
}

#[derive(Clone)]
struct Intrabars {
    key: SecurityKey,
    candles: Vec<Kline>,
}

impl SecurityDataProvider for Intrabars {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        (*key == self.key).then(|| self.candles.clone())
    }
}

/// ETHUSDT 4h candles closing 10, 20 and 30 over three periods of a 1h chart.
#[derive(Clone)]
struct EthFourHour;

impl SecurityDataProvider for EthFourHour {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        (key.symbol == "ETHUSDT" && key.interval == 14_400.0).then(|| {
            [10.0, 20.0, 30.0]
                .iter()
                .enumerate()
                .map(|(period, close)| Kline {
                    open_time: period as i64 * 4 * HOUR,
                    close_time: (period as i64 + 1) * 4 * HOUR - 1,
                    open: *close,
                    high: *close,
                    low: *close,
                    close: *close,
                    volume: 1.0,
                })
                .collect()
        })
    }
}

pub(super) fn session_with(
    source: &str,
    provider: Option<Box<dyn SecurityDataProvider>>,
) -> Session {
    let program = compiler::compile(source, limits(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    Session::new(
        program,
        HashMap::new(),
        limits(),
        Some(0.01),
        symbol(),
        provider,
        None,
    )
}

#[test]
fn another_symbols_period_start_is_marked_when_its_candle_changes() {
    let chart = hourly(12);
    let read = |options: &str| {
        let source = format!(
            "//@version=6\nindicator(\"T\")\nplot(request.security(\"ETHUSDT\", \"240\", close{options}), \"x\")"
        );
        let mut session = session_with(&source, Some(Box::new(EthFourHour)));
        session.evaluate(&chart).expect("script runs");
        plot_values(&session, "x")
    };
    let open = |period: usize| Some(10.0 * (period + 1) as f64);
    let all: Vec<Option<f64>> = (0..12).map(|hour| open(hour / 4)).collect();
    assert_eq!(read(AHEAD), all);
    let gapped: Vec<Option<f64>> = (0..12)
        .map(|hour| (hour % 4 == 0).then(|| open(hour / 4)).flatten())
        .collect();
    assert_eq!(
        read(", gaps = barmerge.gaps_on, lookahead = barmerge.lookahead_on"),
        gapped
    );
}

/// The chart's history up to hour 5 is loaded and hour 6 is a live bar: the periods that history
/// cannot complete are read as they stand, and the live bar's period develops with it.
#[test]
fn a_live_bar_still_reads_its_developing_period() {
    let chart = hourly(7);
    let source = format!(
        "//@version=6\nindicator(\"T\")\nplot({}, \"x\")",
        request("close", AHEAD)
    );
    let mut session = session_with(&source, None);
    session
        .load_history(&chart[..6], true, None)
        .expect("history runs");
    session
        .execute(
            BarEvent {
                candle: chart[6].clone(),
                phase: BarPhase::RealtimeTick { is_new: true },
            },
            true,
            None,
        )
        .expect("the live bar runs");
    let values = plot_values(&session, "x");
    assert_eq!(
        values[0],
        Some(chart[3].close),
        "a period history completed"
    );
    assert_eq!(
        values[5],
        Some(chart[5].close),
        "history knows the period only up to hour 5"
    );
    assert_eq!(
        values[6],
        Some(chart[6].close),
        "the live bar reads the developing period"
    );
}
