//! Where a chart bar ends is its own close, not its nominal length: Pine's month is 30 days and a
//! month has up to 31, so a January bar's intrabars include 31 January and its `time_close` is 1
//! February. And what the provider could not give is asked for again, not remembered as "none".

use crate::model::Kline;
use crate::pine::compiler::NoLibraries;
use crate::pine::compiler::program::SymbolInfo;
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::runtime::session::Session;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};
use crate::pine::syntax::token::Limits;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const JAN_1_2024: i64 = 1_704_067_200_000;
const FEB_1_2024: i64 = JAN_1_2024 + 31 * DAY;
const MAR_1_2024: i64 = FEB_1_2024 + 29 * DAY;
const APR_1_2024: i64 = MAR_1_2024 + 31 * DAY;

fn bar(open_time: i64, close_time: i64) -> Kline {
    Kline {
        open_time,
        close_time,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
    }
}

fn symbol() -> SymbolInfo {
    SymbolInfo {
        ticker: "BTCUSDT".to_owned(),
        ticker_id: "BINANCE:BTCUSDT".to_owned(),
        ..SymbolInfo::default()
    }
}

/// A candle of `length` from `start` up to `end`.
fn candles(start: i64, end: i64, length: i64) -> Vec<Kline> {
    (0..(end - start) / length)
        .map(|index| bar(start + index * length, start + (index + 1) * length - 1))
        .collect()
}

struct Fixed(Vec<Kline>);

impl SecurityDataProvider for Fixed {
    fn candles(&self, _: &SecurityKey) -> Option<Vec<Kline>> {
        Some(self.0.clone())
    }
}

#[test]
fn a_month_of_31_days_takes_its_last_day_and_closes_on_the_first() {
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(time_close)\n\
        plot(array.size(request.security_lower_tf(syminfo.tickerid, \"1D\", close)))";
    let months = vec![
        bar(JAN_1_2024, FEB_1_2024 - 1),
        bar(FEB_1_2024, MAR_1_2024 - 1),
        bar(MAR_1_2024, APR_1_2024 - 1),
    ];
    let mut chart = ExecutionController::new(
        source,
        HashMap::new(),
        symbol(),
        None,
        Limits::default(),
        &NoLibraries,
        Some(Box::new(Fixed(candles(JAN_1_2024, APR_1_2024, DAY)))),
        None,
    )
    .expect("compiles")
    .with_timeframe_seconds(2_592_000.0);
    let update = chart.update(&months, None, APR_1_2024 + DAY);
    assert!(update.error.is_none(), "{:?}", update.error);
    let plot = |index: usize| -> Vec<f64> {
        update.output.plots[index]
            .values
            .iter()
            .map(|value| value.expect("a value on every bar"))
            .collect()
    };
    assert_eq!(
        plot(0),
        [FEB_1_2024, MAR_1_2024, APR_1_2024].map(|ms| ms as f64)
    );
    assert_eq!(plot(1), [31.0, 29.0, 31.0]);
}

/// A provider that gives nothing for its first `failing` asks and 30-minute candles after.
struct Flaky {
    asked: Arc<AtomicUsize>,
    failing: usize,
    gives: Option<Vec<Kline>>,
}

impl SecurityDataProvider for Flaky {
    fn candles(&self, _: &SecurityKey) -> Option<Vec<Kline>> {
        if self.asked.fetch_add(1, Ordering::SeqCst) < self.failing {
            None
        } else {
            self.gives.clone()
        }
    }
}

/// Three hours of one-hour bars run against `provider`, as the sizes of their intrabars.
fn intrabar_counts(session: &mut Session, rebuilds: usize) -> Vec<Vec<f64>> {
    let bars = candles(0, 3 * HOUR, HOUR);
    (0..rebuilds)
        .map(|_| {
            session
                .load_history(&bars, None, None)
                .expect("history runs");
            session.output().plots[0]
                .values
                .iter()
                .map(|value| value.expect("a value on every bar"))
                .collect()
        })
        .collect()
}

fn session(asked: &Arc<AtomicUsize>, failing: usize, gives: Option<Vec<Kline>>) -> Session {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\n\
         plot(array.size(request.security_lower_tf(syminfo.tickerid, \"30\", close)))",
        Limits::default(),
        &NoLibraries,
    );
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol(),
        Some(Box::new(Flaky {
            asked: Arc::clone(asked),
            failing,
            gives,
        })),
        None,
    );
    session.configured_bar_seconds = Some(3_600.0);
    session
}

#[test]
fn a_series_the_provider_could_not_give_is_asked_for_again_after_the_window() {
    let asked = Arc::new(AtomicUsize::new(0));
    let mut run = session(&asked, 1, Some(candles(0, 3 * HOUR, HOUR / 2)));
    run.security_retry_after = Duration::ZERO;
    // The first bar finds nothing; the provider has recovered for the second, which a window of
    // nothing allows to ask.
    assert_eq!(intrabar_counts(&mut run, 1), [[0.0, 2.0, 2.0]]);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn inside_the_window_it_is_left_alone_and_a_rebuild_asks_at_once() {
    let asked = Arc::new(AtomicUsize::new(0));
    let mut run = session(&asked, 1, Some(candles(0, 3 * HOUR, HOUR / 2)));
    assert_eq!(
        intrabar_counts(&mut run, 2),
        [[0.0, 0.0, 0.0], [2.0, 2.0, 2.0]],
        "a failing provider is asked once a run, and again by the next"
    );
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_series_the_provider_answered_empty_is_an_answer_and_is_asked_once_a_run() {
    let asked = Arc::new(AtomicUsize::new(0));
    let mut run = session(&asked, 0, Some(Vec::new()));
    run.security_retry_after = Duration::ZERO;
    assert_eq!(intrabar_counts(&mut run, 1), [[0.0; 3]]);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "three bars, one question");
    // A rebuild is a new run: the provider is asked again, since the chart it answered for may
    // have grown or a live candle moved on (what it gave is not kept across a reload).
    assert_eq!(intrabar_counts(&mut run, 1), [[0.0; 3]]);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}
