//! A `request.security` call under a condition sees the same higher-timeframe bars as one that
//! always runs: TradingView evaluates the expression on every bar, and the condition only picks
//! whether its value is used (audit item 22).

use super::bench::run_script;
use crate::model::Kline;

const HOUR: i64 = 3_600_000;

/// Hourly bars with a wandering close and a range around it, so a bar left out of a 4-hour
/// candle changes its high, low or close.
fn hourly(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let wave = ((index * 37) % 23) as f64 + (index as f64 * 0.3).sin() * 5.0;
            let close = 100.0 + wave;
            Kline {
                open_time: index as i64 * HOUR,
                close_time: (index as i64 + 1) * HOUR - 1,
                open: close - 1.0,
                high: close + 2.0 + (index % 5) as f64,
                low: close - 2.0 - (index % 3) as f64,
                close,
                volume: 1.0 + (index % 7) as f64,
            }
        })
        .collect()
}

/// The values of the plots `always` and `gated` of `condition ? request.security(...) : always`.
fn plots(condition: &str, expression: &str, extra: &str) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let source = format!(
        "//@version=6
indicator(\"Conditional security\")
float always = request.security(syminfo.tickerid, \"240\", {expression}{extra})
float gated = {condition} ? request.security(syminfo.tickerid, \"240\", {expression}{extra}) : always
plot(always, \"always\")
plot(gated, \"gated\")
"
    );
    let session = run_script(&source, &hourly(120), false, |_| {});
    let output = session.output();
    let find = |title: &str| {
        output
            .plots
            .iter()
            .find(|plot| plot.title.as_deref() == Some(title))
            .map(|plot| plot.values.clone())
            .expect("plot")
    };
    (find("always"), find("gated"))
}

fn assert_same(condition: &str, expression: &str, extra: &str) {
    let (always, gated) = plots(condition, expression, extra);
    assert!(always.iter().flatten().count() > 20, "the run has values");
    for (index, (a, g)) in always.iter().zip(&gated).enumerate() {
        match (a, g) {
            (Some(a), Some(g)) => {
                assert!(
                    (a - g).abs() < 1e-9,
                    "bar {index}: {a} vs {g} for {condition}"
                )
            }
            _ => assert_eq!(a, g, "bar {index} for {condition}"),
        }
    }
}

#[test]
fn a_call_that_skips_some_bars_matches_one_that_runs_on_all() {
    // Skips the last bar of every second 4-hour bucket (index 7, 19, ...) and others besides.
    assert_same("bar_index % 3 != 1", "ta.ema(close, 3)", "");
}

#[test]
fn a_call_that_skips_the_high_low_and_volume_of_a_candle_still_sees_them() {
    assert_same(
        "bar_index % 2 == 0",
        "ta.highest(high, 3) - ta.lowest(low, 3)",
        "",
    );
    assert_same("bar_index % 2 == 0", "ta.sma(volume, 2)", "");
}

#[test]
fn a_call_first_reached_late_builds_the_history_from_the_first_bar() {
    assert_same("bar_index > 37", "ta.ema(close, 4)", "");
}

#[test]
fn a_call_on_every_bar_is_unchanged() {
    assert_same("true", "ta.rsi(close, 5)", "");
}
