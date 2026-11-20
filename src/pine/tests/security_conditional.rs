//! A `request.security` call under a condition sees the same higher-timeframe bars as one that
//! always runs: TradingView evaluates the expression on every bar, and the condition only picks
//! whether its value is used (audit item 22).

use super::bench::run_script;
use crate::bars::Kline;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;

/// Bars at `times` (milliseconds) with a wandering close and a range around it, so a bar left out
/// of a higher-timeframe candle changes its high, low, close or volume.
fn bars_at(times: &[i64], spacing: i64) -> Vec<Kline> {
    times
        .iter()
        .enumerate()
        .map(|(index, time)| {
            let wave = ((index * 37) % 23) as f64 + (index as f64 * 0.3).sin() * 5.0;
            let close = 100.0 + wave;
            Kline {
                open_time: *time,
                close_time: time + spacing - 1,
                open: close - 1.0,
                high: close + 2.0 + (index % 5) as f64,
                low: close - 2.0 - (index % 3) as f64,
                close,
                volume: 1.0 + (index % 7) as f64,
            }
        })
        .collect()
}

fn hourly(count: usize) -> Vec<Kline> {
    let times: Vec<i64> = (0..count as i64).map(|index| index * HOUR).collect();
    bars_at(&times, HOUR)
}

/// The values of the plots `always` and `gated`, for `request` (the whole call, written as the
/// script should) run on every bar and under `condition`.
fn plots(
    bars: &[Kline],
    security: bool,
    prelude: &str,
    condition: &str,
    request: &str,
) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let source = format!(
        "//@version=6
indicator(\"Conditional security\")
{prelude}
float always = {request}
float gated = {condition} ? {request} : always
plot(always, \"always\")
plot(gated, \"gated\")
"
    );
    let session = run_script(&source, bars, security, |_| {});
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

fn assert_equal(always: &[Option<f64>], gated: &[Option<f64>], what: &str) {
    assert!(always.iter().flatten().count() > 10, "the run has values");
    for (index, (a, g)) in always.iter().zip(gated).enumerate() {
        match (a, g) {
            (Some(a), Some(g)) => {
                assert!((a - g).abs() < 1e-9, "bar {index}: {a} vs {g} for {what}")
            }
            _ => assert_eq!(a, g, "bar {index} for {what}"),
        }
    }
}

fn assert_same(condition: &str, expression: &str) {
    let request = format!("request.security(syminfo.tickerid, \"240\", {expression})");
    let (always, gated) = plots(&hourly(120), false, "", condition, &request);
    assert_equal(&always, &gated, condition);
}

#[test]
fn a_call_that_skips_some_bars_matches_one_that_runs_on_all() {
    // Skips the last bar of every second 4-hour bucket (index 7, 19, ...) and others besides.
    assert_same("bar_index % 3 != 1", "ta.ema(close, 3)");
}

#[test]
fn a_call_that_skips_the_high_low_and_volume_of_a_candle_still_sees_them() {
    assert_same(
        "bar_index % 2 == 0",
        "ta.highest(high, 3) - ta.lowest(low, 3)",
    );
    assert_same("bar_index % 2 == 0", "ta.sma(volume, 2)");
}

#[test]
fn a_call_first_reached_late_builds_the_history_from_the_first_bar() {
    assert_same("bar_index > 37", "ta.ema(close, 4)");
}

#[test]
fn a_call_on_every_bar_is_unchanged() {
    assert_same("true", "ta.rsi(close, 5)");
}

#[test]
fn a_request_inside_a_request_replays_the_enclosing_candles_not_the_charts() {
    let nested =
        "request.security(syminfo.tickerid, \"240\", request.security(\"\", \"240\", volume[1]))";
    let direct = "request.security(syminfo.tickerid, \"240\", volume[1])";
    let bars = hourly(120);
    let (always, gated) = plots(&bars, false, "", "bar_index % 3 != 1", nested);
    assert_equal(&always, &gated, "a gated nested request");
    // Run on every bar, the nested form reads the same series as the direct one.
    let (direct_always, _) = plots(&bars, false, "", "true", direct);
    assert_equal(&direct_always, &always, "nested against direct");
}

#[test]
fn a_gated_expression_reads_chart_variables_as_of_each_skipped_bar() {
    // `foreign` comes from a request statement, so no slice recomputes it: the expression reads
    // the chart's value, which for a skipped bar is that bar's, not today's.
    let request = "request.security(syminfo.tickerid, \"240\", ta.sma(foreign, 3))";
    let prelude = "float foreign = request.security(\"BINANCE:ETHUSDT\", \"60\", close)";
    let (always, gated) = plots(&hourly(120), true, prelude, "bar_index % 3 != 1", request);
    assert_equal(&always, &gated, "a chart variable read by a gated request");
}

#[test]
fn a_gap_in_the_chart_does_not_close_a_replayed_candle_twice() {
    // Minute bars with a hole at minute 4: the bar after it is two minutes after the one before.
    let times: Vec<i64> = (0..80i64)
        .filter(|minute| *minute != 4)
        .map(|minute| minute * MINUTE)
        .collect();
    let bars = bars_at(&times, MINUTE);
    let request = "request.security(syminfo.tickerid, \"4\", ta.sma(close, 2) + volume)";
    let (always, gated) = plots(&bars, false, "", "bar_index > 3", request);
    assert_equal(&always, &gated, "a request resumed after a gap");
}

#[test]
fn a_request_spelled_with_spaces_is_found_too() {
    let request = "request . security(syminfo.tickerid, \"240\", ta.sma(volume, 2))";
    let (always, gated) = plots(&hourly(120), false, "", "bar_index % 2 == 0", request);
    assert_equal(&always, &gated, "a request written with spaces");
}
