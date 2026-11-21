//! A `ta.*` built-in against the formula TradingView documents for it, written out in Pine from the
//! definition. Each test plots both and compares them bar by bar (the same bars, `na` where the
//! other is `na`), on a random walk, so a built-in that drifts from its definition fails here.

use super::bench::{bars, run_script};
use super::varip::live_bar;

/// The plotted series of `source`, one `Vec` per `plot`, over `count` one-minute bars.
pub(super) fn plots(source: &str, count: usize) -> Vec<Vec<Option<f64>>> {
    let data = bars(count, 60_000);
    let session = run_script(source, &data, false, |_| {});
    session
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect()
}

/// Asserts two series agree: both `na` on the same bars, and within `tolerance` elsewhere.
pub(super) fn assert_same(label: &str, built_in: &[Option<f64>], reference: &[Option<f64>]) {
    assert_eq!(built_in.len(), reference.len(), "{label}: length");
    for (bar, (a, b)) in built_in.iter().zip(reference).enumerate() {
        match (a, b) {
            (None, None) => {}
            (Some(a), Some(b)) => assert!(
                (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                "{label}: bar {bar}: built-in {a} against reference {b}"
            ),
            _ => panic!("{label}: bar {bar}: built-in {a:?} against reference {b:?}"),
        }
    }
}

fn first_value(series: &[Option<f64>]) -> Option<usize> {
    series.iter().position(Option::is_some)
}

#[test]
fn macd_follows_its_lengths() {
    let plotted = plots(
        "//@version=6
indicator(\"Macd\")
[line, signal, hist] = ta.macd(close, 5, 35, 5)
fast = ta.ema(close, 5)
slow = ta.ema(close, 35)
reference = fast - slow
plot(line)
plot(signal)
plot(hist)
plot(reference)
plot(ta.ema(reference, 5))
plot(reference - ta.ema(reference, 5))
",
        1_500,
    );
    assert_same("line", &plotted[0], &plotted[3]);
    assert_same("signal", &plotted[1], &plotted[4]);
    assert_same("histogram", &plotted[2], &plotted[5]);
    // The slow EMA seeds on its 35th value; the signal on the fifth value of the line.
    assert_eq!(first_value(&plotted[0]), Some(34));
    assert_eq!(first_value(&plotted[1]), Some(38));
}

#[test]
fn macd_with_the_default_lengths_matches_the_ema_composition() {
    let plotted = plots(
        "//@version=6
indicator(\"Macd\")
[line, signal, hist] = ta.macd(close, 12, 26, 9)
plot(line)
plot(ta.ema(close, 12) - ta.ema(close, 26))
plot(signal)
plot(ta.ema(ta.ema(close, 12) - ta.ema(close, 26), 9))
",
        800,
    );
    assert_same("line", &plotted[0], &plotted[1]);
    assert_same("signal", &plotted[2], &plotted[3]);
}

/// TradingView's printed reference for `ta.supertrend`, as a function the script calls.
const SUPERTREND_REFERENCE: &str = "
f_supertrend(float factor, simple int atrPeriod) =>
    src = hl2
    atr = ta.atr(atrPeriod)
    upperBand = src + factor * atr
    lowerBand = src - factor * atr
    prevLowerBand = nz(lowerBand[1])
    prevUpperBand = nz(upperBand[1])
    lowerBand := lowerBand > prevLowerBand or close[1] < prevLowerBand ? lowerBand : prevLowerBand
    upperBand := upperBand < prevUpperBand or close[1] > prevUpperBand ? upperBand : prevUpperBand
    int direction = na
    float superTrend = na
    prevSuperTrend = superTrend[1]
    if na(atr[1])
        direction := 1
    else if prevSuperTrend == prevUpperBand
        direction := close > upperBand ? -1 : 1
    else
        direction := close < lowerBand ? 1 : -1
    superTrend := direction == -1 ? lowerBand : upperBand
    [superTrend, direction]
";

#[test]
fn supertrend_equals_the_reference_code_from_the_first_bar() {
    let source = format!(
        "//@version=6
indicator(\"Supertrend\", overlay = true)
{SUPERTREND_REFERENCE}
[line, direction] = ta.supertrend(3, 10)
[expected, expectedDirection] = f_supertrend(3, 10)
plot(line)
plot(expected)
plot(direction)
plot(expectedDirection)
"
    );
    let plotted = plots(&source, 2_000);
    assert_same("line", &plotted[0], &plotted[1]);
    assert_same("direction", &plotted[2], &plotted[3]);
    assert_eq!(
        plotted[0][0],
        Some(0.0),
        "the first bar is 0 on TradingView"
    );
    assert!(plotted[0][1].is_none());
    assert_eq!(first_value(&plotted[0][1..]), Some(8), "real from bar 9");
}

#[test]
fn supertrend_works_inside_a_request() {
    let source = format!(
        "//@version=6
indicator(\"Supertrend\", overlay = true)
{SUPERTREND_REFERENCE}
[line, direction] = request.security(syminfo.tickerid, \"5\", ta.supertrend(3, 10))
[expected, expectedDirection] = request.security(syminfo.tickerid, \"5\", f_supertrend(3, 10))
plot(line)
plot(expected)
plot(direction)
plot(expectedDirection)
"
    );
    let data = bars(1_500, 60_000);
    let session = run_script(&source, &data, true, |_| {});
    let plotted: Vec<Vec<Option<f64>>> = session
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect();
    assert_same("line", &plotted[0], &plotted[1]);
    assert_same("direction", &plotted[2], &plotted[3]);
    assert!(plotted[0].iter().flatten().count() > 100);
}

#[test]
fn named_arguments_of_supertrend_and_sar_bind_by_name() {
    let plotted = plots(
        "//@version=6
indicator(\"Named\", overlay = true)
[line, direction] = ta.supertrend(3, 10)
[swapped, swappedDirection] = ta.supertrend(atrPeriod = 10, factor = 3)
[named, namedDirection] = ta.supertrend(factor = 3, atrPeriod = 10)
plot(line)
plot(swapped)
plot(named)
plot(direction)
plot(swappedDirection)
plot(namedDirection)
plot(ta.sar(0.02, 0.03, 0.3))
plot(ta.sar(max = 0.3, inc = 0.03, start = 0.02))
plot(ta.sar())
plot(ta.sar(0.02, 0.02, 0.2))
",
        2_000,
    );
    assert_same("supertrend, reordered", &plotted[0], &plotted[1]);
    assert_same("supertrend, named in order", &plotted[0], &plotted[2]);
    assert_same("direction, reordered", &plotted[3], &plotted[4]);
    assert_same("direction, named in order", &plotted[3], &plotted[5]);
    assert_same("sar, reordered", &plotted[6], &plotted[7]);
    assert_same("sar defaults", &plotted[8], &plotted[9]);
}

#[test]
fn the_vwap_variable_equals_vwap_of_hlc3_and_can_be_read_more_than_once() {
    let plotted = plots(
        "//@version=6
indicator(\"Vwap\", overlay = true)
plot(ta.vwap)
plot(ta.vwap(hlc3))
plot(ta.vwap + 0)
plot(ta.vwap[1])
plot(ta.vwap(hlc3)[1])
",
        3_000,
    );
    assert_same("variable", &plotted[0], &plotted[1]);
    assert_same("second read", &plotted[0], &plotted[2]);
    assert_same("history", &plotted[3], &plotted[4]);
    assert!(plotted[0].iter().flatten().count() > 2_900);
}

#[test]
fn a_vwap_variable_read_only_now_and_then_still_covers_every_bar() {
    let plotted = plots(
        "//@version=6
indicator(\"Vwap\", overlay = true)
every = ta.vwap(hlc3)
sometimes = bar_index % 7 == 6 or bar_index == 9
plot(sometimes ? ta.vwap : na)
plot(sometimes ? every : na)
late = bar_index == 40
plot(late ? ta.vwap : na)
plot(late ? every : na)
",
        400,
    );
    assert_same("every seventh bar", &plotted[0], &plotted[1]);
    assert_same("first read on bar 40", &plotted[2], &plotted[3]);
    assert!(plotted[0].iter().flatten().count() > 50);
}

#[test]
fn the_vwap_variable_works_where_a_script_may_put_it() {
    // Alone in the script (nothing else reads hlc3 or the function): against a run of the function.
    let alone = plots(
        "//@version=6
indicator(\"Alone\", overlay = true)
plot(bar_index % 5 == 4 ? ta.vwap : na)
plot(ta.vwap)
",
        600,
    );
    let reference = plots(
        "//@version=6
indicator(\"Reference\", overlay = true)
v = ta.vwap(hlc3)
plot(bar_index % 5 == 4 ? v : na)
plot(v)
",
        600,
    );
    assert_same("sparse alone", &alone[0], &reference[0]);
    assert_same("every bar alone", &alone[1], &reference[1]);
    // TradingView: a value from bar 0 (the bar's own hlc3), never na while it warms up.
    assert!(alone[1][0].is_some());

    // Through a user function, a variable declaration, history and a higher timeframe.
    let plotted = plots(
        "//@version=6
indicator(\"Places\", overlay = true)
f() => ta.vwap
var float kept = na
kept := ta.vwap
v = ta.vwap(hlc3)
plot(f())
plot(v)
plot(kept)
plot(v)
plot(ta.vwap[3])
plot(v[3])
plot(request.security(syminfo.tickerid, \"5\", ta.vwap))
plot(request.security(syminfo.tickerid, \"5\", ta.vwap(hlc3)))
plot(math.max(ta.vwap, close) - math.max(v, close))
plot(0.0)
",
        1_500,
    );
    assert_same("user function", &plotted[0], &plotted[1]);
    assert_same("var declaration", &plotted[2], &plotted[3]);
    assert_same("history", &plotted[4], &plotted[5]);
    assert_same("security", &plotted[6], &plotted[7]);
    assert_same("operand", &plotted[8], &plotted[9]);
}

#[test]
fn the_cumulative_variables_keep_a_state_each() {
    // Read together, every variable must give what it gives alone: they share one table of
    // states, so a slot taken by two of them would show up here.
    let alone = |variable: &str| {
        plots(
            &format!("//@version=6\nindicator(\"One\")\nplot({variable})\n"),
            800,
        )
        .remove(0)
    };
    let names = crate::pine::builtins::ta::extra::VARIABLES.map(|variable| variable.name);
    let together = plots(
        &format!(
            "//@version=6\nindicator(\"All\")\n{}",
            names
                .iter()
                .map(|name| format!("plot({name})\n"))
                .collect::<String>()
        ),
        800,
    );
    for (name, plotted) in names.iter().zip(&together) {
        assert_same(name, plotted, &alone(name));
    }
}

#[test]
fn stdev_and_variance_honour_biased() {
    let plotted = plots(
        "//@version=6
indicator(\"Dispersion\")
n = 20
sampleVariance = ta.variance(close, n) * n / (n - 1)
plot(ta.variance(close, n, false))
plot(sampleVariance)
plot(ta.stdev(close, n, false))
plot(math.sqrt(sampleVariance))
plot(ta.stdev(close, n))
plot(ta.stdev(close, n, true))
plot(ta.variance(close, 1, false))
",
        600,
    );
    assert_same("variance", &plotted[0], &plotted[1]);
    assert_same("stdev", &plotted[2], &plotted[3]);
    assert_same("stdev is biased by default", &plotted[4], &plotted[5]);
    assert!(
        plotted[6].iter().all(Option::is_none),
        "one value has no sample variance"
    );
}

#[test]
fn bbw_is_the_band_width_in_percent() {
    let plotted = plots(
        "//@version=6
indicator(\"Bbw\")
[basis, upper, lower] = ta.bb(close, 20, 2)
plot(ta.bbw(close, 20, 2))
plot((upper - lower) / basis * 100)
",
        600,
    );
    assert_same("bbw", &plotted[0], &plotted[1]);
}

#[test]
fn iii_is_the_intensity_of_each_bar_not_a_running_total() {
    let plotted = plots(
        "//@version=6
indicator(\"Iii\")
plot(ta.iii)
plot(high == low ? na : ((2 * close - high - low) / (high - low)) * volume)
",
        600,
    );
    assert_same("iii", &plotted[0], &plotted[1]);
}

#[test]
fn obv_pvt_and_cum_start_on_the_second_bar_like_their_reference_code() {
    let plotted = plots(
        "//@version=6
indicator(\"Cumulative\")
plot(ta.obv)
plot(ta.cum(math.sign(ta.change(close)) * volume))
plot(ta.pvt)
plot(ta.cum((ta.change(close) / close[1]) * volume))
plot(ta.cum(bar_index < 3 ? na : 1))
",
        600,
    );
    assert_same("obv", &plotted[0], &plotted[1]);
    assert_same("pvt", &plotted[2], &plotted[3]);
    assert_eq!(first_value(&plotted[0]), Some(1));
    assert_eq!(first_value(&plotted[2]), Some(1));
    assert_eq!(plotted[4][2], None, "cum of a leading na is na");
    assert_eq!(plotted[4][3], Some(1.0));
}

#[test]
fn stdev_and_variance_take_named_and_positional_arguments_alike() {
    let plotted = plots(
        "//@version=6
indicator(\"Named dispersion\")
plot(ta.stdev(close, 20, false))
plot(ta.stdev(source = close, length = 20, biased = false))
plot(ta.stdev(length = 20, source = close, biased = false))
plot(ta.variance(close, 20))
plot(ta.variance(close, length = 20))
plot(ta.variance(close, 20, true))
plot(ta.variance(close, biased = true, length = 20))
",
        600,
    );
    assert_same("stdev named", &plotted[0], &plotted[1]);
    assert_same("stdev reordered", &plotted[0], &plotted[2]);
    assert_same("variance mixed", &plotted[3], &plotted[4]);
    assert_same("variance explicit biased", &plotted[3], &plotted[5]);
    assert_same("variance named biased", &plotted[3], &plotted[6]);
}

#[test]
fn the_volume_variables_follow_a_live_bar_like_their_formulas() {
    // The bar being drawn is recomputed on every tick and folded for good only when it closes: the
    // variable must give what the formula written out in Pine gives at each of the four stages
    // (the history, the live bar's first tick, a changed tick and its close).
    let seen = live_bar(
        "plot(ta.obv)
plot(ta.cum(math.sign(ta.change(close)) * volume))
plot(ta.pvt)
plot(ta.cum(ta.change(close) / close[1] * volume))
plot(ta.accdist)
plot(ta.cum(close == high and close == low or high == low ? 0.0 : (2 * close - low - high) / (high - low) * volume))
plot(ta.iii)
plot((2 * close - high - low) / (high - low) * volume)
plot(ta.wvad)
plot((close - open) / (high - low) * volume)",
    );
    assert_eq!(seen.len(), 4);
    for (stage, plotted) in seen.iter().enumerate() {
        for (name, pair) in ["obv", "pvt", "accdist", "iii", "wvad"]
            .iter()
            .zip(plotted.chunks(2))
        {
            assert!(
                (pair[0] - pair[1]).abs() <= 1e-9 * pair[0].abs().max(pair[1].abs()).max(1.0),
                "stage {stage}: {name} {} against its formula {}",
                pair[0],
                pair[1]
            );
        }
    }
    // The changed tick moved the close, so a total that depends on its size moved with it, and the
    // close kept that value.
    assert_ne!(seen[1][2], seen[2][2], "pvt follows the changed tick");
    assert_eq!(
        seen[2], seen[3],
        "the close folds what the last tick showed"
    );
}

#[test]
fn stdev_and_variance_over_a_window_of_one() {
    let plotted = plots(
        "//@version=6
indicator(\"One\")
plot(ta.stdev(close, 1))
plot(ta.variance(close, 1))
plot(ta.stdev(close, 1, false))
plot(ta.variance(close, 1, false))
plot(ta.stdev(close, 2, false))
",
        50,
    );
    assert!(
        plotted[0].iter().all(|v| *v == Some(0.0)),
        "population of one is 0"
    );
    assert!(plotted[1].iter().all(|v| *v == Some(0.0)));
    assert!(
        plotted[2].iter().all(Option::is_none),
        "no sample figure of one value"
    );
    assert!(plotted[3].iter().all(Option::is_none));
    assert_eq!(
        first_value(&plotted[4]),
        Some(1),
        "a sample needs two values"
    );
}

#[test]
fn stdev_and_variance_with_a_length_that_is_not_a_window_are_na() {
    let plotted = plots(
        "//@version=6
indicator(\"None\")
plot(ta.stdev(close, 0))
plot(ta.stdev(close, -3, false))
plot(ta.variance(close, 0, false))
plot(ta.variance(close, -1))
",
        50,
    );
    for series in &plotted {
        assert!(series.iter().all(Option::is_none));
    }
}

#[test]
fn the_sample_figure_is_the_population_one_scaled_wherever_the_window_is_complete() {
    // A source with gaps: both are na on the same bars (a window with a missing value), and the
    // ratio is the same wherever they have a value.
    let plotted = plots(
        "//@version=6
indicator(\"Gaps\")
source = bar_index % 17 == 3 ? na : close
n = 10
plot(ta.stdev(source, n, false))
plot(ta.stdev(source, n) * math.sqrt(n / (n - 1.0)))
plot(ta.variance(source, n, false))
plot(ta.variance(source, n) * n / (n - 1.0))
",
        400,
    );
    assert_same("stdev", &plotted[0], &plotted[1]);
    assert_same("variance", &plotted[2], &plotted[3]);
    assert!(plotted[0].iter().flatten().count() > 50);
    assert!(plotted[0].iter().any(Option::is_none));
}

#[test]
fn stdev_and_variance_work_where_a_script_may_put_them() {
    let plotted = plots(
        "//@version=6
indicator(\"Places\", overlay = true)
f(x) => ta.stdev(x, 20, false)
g(x, n) => ta.variance(x, n, false)
s = ta.stdev(close, 20, false)
v = ta.variance(close, 20, false)
plot(f(close))
plot(s)
plot(g(close, 20))
plot(v)
plot(ta.stdev(close, 20, false)[3])
plot(s[3])
plot(ta.stdev(close[2], 20, false))
plot(s[2])
plot(bar_index % 2 == 0 ? ta.stdev(close, 20, false)[1] : na)
plot(bar_index % 2 == 0 ? s[1] : na)
plot(bar_index % 3 == 0 ? ta.variance(close, 20, false)[1] : na)
plot(bar_index % 3 == 0 ? v[1] : na)
plot(ta.stdev(source = close, length = 20, biased = false))
plot(ta.stdev(length = 20, source = close, biased = false))
",
        600,
    );
    assert_same("user function", &plotted[0], &plotted[1]);
    assert_same("two arguments of a function", &plotted[2], &plotted[3]);
    assert_same("history", &plotted[4], &plotted[5]);
    assert_same("history of the source", &plotted[6], &plotted[7]);
    assert_same("run ahead of a ternary arm", &plotted[8], &plotted[9]);
    assert_same(
        "variance run ahead of a ternary arm",
        &plotted[10],
        &plotted[11],
    );
    assert_same("named arguments", &plotted[12], &plotted[1]);
    assert_same("reordered arguments", &plotted[13], &plotted[1]);
}

#[test]
fn stdev_with_biased_false_works_inside_a_request() {
    let data = bars(1_500, 60_000);
    let session = run_script(
        "//@version=6
indicator(\"Request\", overlay = true)
a = request.security(syminfo.tickerid, \"5\", ta.stdev(close, 20, false))
b = request.security(syminfo.tickerid, \"5\", ta.stdev(close, 20) * math.sqrt(20 / 19.0))
plot(a)
plot(b)
",
        &data,
        true,
        |_| {},
    );
    let plotted: Vec<Vec<Option<f64>>> = session
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect();
    assert_same("request", &plotted[0], &plotted[1]);
    assert!(plotted[0].iter().flatten().count() > 100);
}

#[test]
fn the_bundled_obv_starts_on_the_second_bar_and_its_signal_on_the_twenty_second() {
    // `ta.obv` is na on the first bar, as on TradingView, so the 21-bar average of it needs bars 1
    // to 21 and is first real on bar 21.
    let obv = crate::model::pine::bundled::find("obv").expect("the bundled OBV");
    let plotted = plots(obv.source, 400);
    assert_eq!(plotted.len(), 2, "the line and its signal");
    assert_eq!(plotted[0][0], None);
    assert_eq!(first_value(&plotted[0]), Some(1));
    assert_eq!(first_value(&plotted[1]), Some(21));
    assert!(plotted[0][1..].iter().all(Option::is_some));
    assert!(plotted[1][21..].iter().all(Option::is_some));
}

#[test]
fn every_bundled_indicator_still_runs_and_draws() {
    for indicator in &crate::model::pine::bundled::ALL {
        let data = bars(600, 60_000);
        let output = run_script(indicator.source, &data, false, |_| {}).output();
        let has_value = output
            .plots
            .iter()
            .any(|plot| plot.values.iter().any(Option::is_some));
        assert!(
            has_value || !output.markers.is_empty() || !output.arrows.is_empty(),
            "{} draws nothing",
            indicator.id
        );
    }
}

/// TradingView's definition of `ta.tr(handle_na)`, as a function the script calls.
const TRUE_RANGE_REFERENCE: &str = "
f_tr(bool handleNa) =>
    na(close[1]) ? (handleNa ? high - low : na) : math.max(high - low, math.abs(high - close[1]), math.abs(low - close[1]))
";

#[test]
fn true_range_is_na_on_the_first_bar_unless_na_is_handled() {
    let source = format!(
        "//@version=6
indicator(\"True range\")
{TRUE_RANGE_REFERENCE}
plot(ta.tr)
plot(ta.tr(false))
plot(ta.tr(true))
plot(ta.tr(handle_na = true))
plot(ta.tr(bar_index == 0))
plot(f_tr(false))
plot(f_tr(true))
plot(f_tr(bar_index == 0))
"
    );
    let plotted = plots(&source, 600);
    assert_same("variable", &plotted[0], &plotted[5]);
    assert_same("handle_na = false", &plotted[1], &plotted[5]);
    assert_same("handle_na = true", &plotted[2], &plotted[6]);
    assert_same("handle_na named", &plotted[3], &plotted[6]);
    assert_same("handle_na computed", &plotted[4], &plotted[7]);
    assert_eq!(first_value(&plotted[0]), Some(1));
    assert_eq!(first_value(&plotted[2]), Some(0));
}

#[test]
fn atr_and_supertrend_keep_the_first_bars_range() {
    let plotted = plots(
        "//@version=6
indicator(\"Atr\")
plot(ta.atr(14))
plot(ta.rma(ta.tr(true), 14))
",
        600,
    );
    assert_same("atr", &plotted[0], &plotted[1]);
    assert_eq!(first_value(&plotted[0]), Some(13));
}

#[test]
fn keltner_channels_start_on_the_bar_after_their_length_like_the_reference_code() {
    let plotted = plots(
        "//@version=6
indicator(\"Keltner\", overlay = true)
[basis, upper, lower] = ta.kc(close, 20, 1.5, true)
[highLowBasis, highLowUpper, highLowLower] = ta.kc(close, 20, 1.5, false)
average = ta.ema(close, 20)
range = ta.ema(ta.tr, 20)
plot(basis)
plot(upper)
plot(lower)
plot(ta.kcw(close, 20, 1.5, true))
plot(average)
plot(average + 1.5 * range)
plot(average - 1.5 * range)
plot(3 * range / average)
plot(highLowUpper)
",
        800,
    );
    assert_same("basis", &plotted[0], &plotted[4]);
    assert_same("upper", &plotted[1], &plotted[5]);
    assert_same("lower", &plotted[2], &plotted[6]);
    assert_same("width", &plotted[3], &plotted[7]);
    assert_eq!(first_value(&plotted[1]), Some(20), "TradingView: bar 20");
    assert_eq!(
        first_value(&plotted[0]),
        Some(19),
        "the basis is the average itself, a bar before the bands"
    );
    assert_eq!(
        first_value(&plotted[8]),
        Some(19),
        "the high-low form has no first-bar gap"
    );
}

#[test]
fn a_conditional_call_measures_true_range_against_the_charts_previous_close() {
    let plotted = plots(
        "//@version=6
indicator(\"Conditional\")
sometimes = bar_index % 3 == 1
plot(sometimes ? ta.atr(1) : na)
plot(sometimes ? ta.tr(true) : na)
plot(sometimes ? ta.kcw(close, 1, 1, true) : na)
plot(sometimes ? 2 * ta.tr / close : na)
",
        600,
    );
    assert_same("atr", &plotted[0], &plotted[1]);
    assert_same("kcw", &plotted[2], &plotted[3]);
    assert!(plotted[0].iter().flatten().count() > 150);
}

#[test]
fn a_conditional_supertrend_follows_the_reference_code() {
    let source = format!(
        "//@version=6
indicator(\"Conditional supertrend\", overlay = true)
{SUPERTREND_REFERENCE}
sometimes = bar_index % 3 == 1
float line = na
float expected = na
if sometimes
    [built, direction] = ta.supertrend(3, 4)
    line := built
    [wanted, wantedDirection] = f_supertrend(3, 4)
    expected := wanted
plot(line)
plot(expected)
"
    );
    let plotted = plots(&source, 900);
    assert_same("line", &plotted[0], &plotted[1]);
    assert!(plotted[0].iter().flatten().count() > 100);
}

/// The reference code for `ta.dmi`, written out from its definition: the directional movement
/// and the true range smoothed with `ta.rma`, ADX as the smoothed directional index.
const DMI_REFERENCE: &str = "
f_dmi(int len, int smoothing) =>
    up = ta.change(high)
    down = -ta.change(low)
    plusMove = na(up) ? na : (up > down and up > 0 ? up : 0)
    minusMove = na(down) ? na : (down > up and down > 0 ? down : 0)
    trueRange = ta.rma(ta.tr, len)
    plus = 100 * ta.rma(plusMove, len) / trueRange
    minus = 100 * ta.rma(minusMove, len) / trueRange
    total = plus + minus
    adx = 100 * ta.rma(math.abs(plus - minus) / (total == 0 ? 1 : total), smoothing)
    [plus, minus, adx]
";

#[test]
fn dmi_equals_the_reference_code_from_its_first_bar() {
    let source = format!(
        "//@version=6
indicator(\"Dmi\")
{DMI_REFERENCE}
[plus, minus, adx] = ta.dmi(14, 9)
[wantedPlus, wantedMinus, wantedAdx] = f_dmi(14, 9)
plot(plus)
plot(wantedPlus)
plot(minus)
plot(wantedMinus)
plot(adx)
plot(wantedAdx)
"
    );
    let plotted = plots(&source, 1_000);
    assert_same("plus", &plotted[0], &plotted[1]);
    assert_same("minus", &plotted[2], &plotted[3]);
    assert_same("adx", &plotted[4], &plotted[5]);
    assert_eq!(first_value(&plotted[0]), Some(14), "TR starts on bar 1");
    assert_eq!(first_value(&plotted[4]), Some(22));
}

#[test]
fn dmi_shares_no_state_with_the_calls_around_it() {
    // The smoothing states used to sit under `site * 16 + n`, which a later call site could hit.
    let lengths: Vec<usize> = (3..43).collect();
    let mut source = String::from(
        "//@version=6
indicator(\"Neighbours\")
[plus, minus, adx] = ta.dmi(14, 14)
plot(plus)
",
    );
    for length in &lengths {
        source.push_str(&format!("plot(ta.sma(close, {length}))\n"));
    }
    let count = 500;
    let plotted = plots(&source, count);
    let closes: Vec<f64> = bars(count, 60_000).iter().map(|bar| bar.close).collect();
    for (offset, length) in lengths.iter().enumerate() {
        let expected: Vec<Option<f64>> = (0..count)
            .map(|bar| {
                (bar + 1 >= *length)
                    .then(|| closes[bar + 1 - length..=bar].iter().sum::<f64>() / *length as f64)
            })
            .collect();
        assert_same(&format!("sma({length})"), &plotted[offset + 1], &expected);
    }
}

/// The first nine bars of BINANCE:BTCUSDT 5m as TradingView draws them (2026-11-02), and what its
/// `ta.tr`, `ta.atr`, `ta.supertrend`, `ta.kc`, `ta.kcw` and `ta.dmi` logged on them (to three
/// decimals; `None` is `na`).
#[test]
fn the_true_range_family_matches_tradingview_on_its_first_bars() {
    use crate::model::Kline;
    let ohlc: [(f64, f64, f64, f64); 9] = [
        (78354.01, 78523.0, 78354.01, 78454.97),
        (78454.98, 78472.58, 78266.0, 78279.93),
        (78279.94, 78284.01, 78161.49, 78161.49),
        (78161.5, 78223.96, 78044.11, 78055.42),
        (78055.42, 78095.72, 78051.16, 78080.76),
        (78080.77, 78198.01, 78080.76, 78198.01),
        (78198.01, 78214.37, 78054.0, 78059.61),
        (78059.61, 78100.98, 78044.24, 78098.02),
        (78098.02, 78129.33, 78080.0, 78120.01),
    ];
    let data: Vec<Kline> = ohlc
        .iter()
        .enumerate()
        .map(|(i, &(open, high, low, close))| Kline {
            open_time: 1_000_000 + i as i64 * 300_000,
            close_time: 1_000_000 + (i as i64 + 1) * 300_000,
            open,
            high,
            low,
            close,
            volume: 100.0,
        })
        .collect();
    let source = "//@version=6
indicator(\"p670\", overlay = true)
trN = ta.tr
trT = ta.tr(true)
trF = ta.tr(false)
atr3 = ta.atr(3)
[st, dir] = ta.supertrend(2, 3)
[kcm, kcu, kcl] = ta.kc(close, 3, 1.5)
kcw3 = ta.kcw(close, 3, 1.5)
[pdi, mdi, adx] = ta.dmi(3, 3)
plot(trN)
plot(trT)
plot(trF)
plot(atr3)
plot(st)
plot(dir)
plot(kcm)
plot(kcu)
plot(kcl)
plot(kcw3)
plot(pdi)
plot(mdi)
plot(adx)
";
    let session = super::bench::run_script(source, &data, false, |_| {});
    let n = f64::NAN;
    let tradingview: [(&str, [f64; 9]); 13] = [
        (
            "tr",
            [
                n, 206.58, 122.52, 179.85, 44.56, 117.25, 160.37, 56.74, 49.33,
            ],
        ),
        (
            "tr(true)",
            [
                168.99, 206.58, 122.52, 179.85, 44.56, 117.25, 160.37, 56.74, 49.33,
            ],
        ),
        (
            "tr(false)",
            [
                n, 206.58, 122.52, 179.85, 44.56, 117.25, 160.37, 56.74, 49.33,
            ],
        ),
        (
            "atr",
            [
                n, n, 166.03, 170.637, 128.611, 124.824, 136.673, 110.028, 89.796,
            ],
        ),
        (
            "supertrend",
            [
                0.0, n, 78554.81, 78475.308, 78330.662, 78330.662, 78330.662, 78292.667, 78284.256,
            ],
        ),
        ("direction", [1.0; 9]),
        (
            "kc basis",
            [
                n, n, 78298.797, 78177.108, 78128.934, 78163.472, 78111.541, 78104.781, 78112.395,
            ],
        ),
        (
            "kc upper",
            [
                n, n, n, 78431.583, 78289.592, 78331.738, 78315.952, 78249.541, 78221.773,
            ],
        ),
        (
            "kc lower",
            [
                n, n, n, 77922.633, 77968.277, 77995.206, 77907.13, 77960.02, 78003.018,
            ],
        ),
        ("kcw", [n, n, n, 0.007, 0.004, 0.004, 0.005, 0.004, 0.003]),
        ("plus", [n, n, n, 0.0, 0.0, 27.412, 16.667, 13.797, 21.806]),
        (
            "minus",
            [n, n, n, 60.89, 53.822, 36.91, 28.983, 26.954, 22.011],
        ),
        ("adx", [n, n, n, n, n, 71.589, 56.719, 48.574, 32.539]),
    ];
    let output = session.output();
    for ((name, wanted), plot) in tradingview.iter().zip(&output.plots) {
        for (bar, (&wanted, got)) in wanted.iter().zip(&plot.values).enumerate() {
            match got {
                None => assert!(wanted.is_nan(), "{name}, bar {bar}: na against {wanted}"),
                Some(got) => assert!(
                    (got - wanted).abs() <= 6e-4,
                    "{name}, bar {bar}: {got} against TradingView's {wanted}"
                ),
            }
        }
    }
}

#[test]
fn the_true_range_family_follows_a_live_bar_like_its_reference_code() {
    // The live bar is recomputed on every tick against the chart's close[1], and folded into the
    // averages only when it closes: each built-in must give what the formula written out in Pine
    // gives at the history, the first tick, a changed tick and the close.
    let source = format!(
        "{TRUE_RANGE_REFERENCE}{SUPERTREND_REFERENCE}{DMI_REFERENCE}
plot(ta.tr)
plot(f_tr(false))
plot(ta.tr(true))
plot(f_tr(true))
plot(ta.atr(2))
plot(ta.rma(f_tr(true), 2))
[basis, upper, lower] = ta.kc(close, 2, 1.5)
plot(basis)
plot(ta.ema(close, 2))
plot(upper)
plot(ta.ema(close, 2) + 1.5 * ta.ema(f_tr(false), 2))
plot(lower)
plot(ta.ema(close, 2) - 1.5 * ta.ema(f_tr(false), 2))
[line, direction] = ta.supertrend(2, 2)
[wantedLine, wantedDirection] = f_supertrend(2, 2)
plot(line)
plot(wantedLine)
plot(direction)
plot(wantedDirection)
[plus, minus, adx] = ta.dmi(2, 2)
[wantedPlus, wantedMinus, wantedAdx] = f_dmi(2, 2)
plot(plus)
plot(wantedPlus)
plot(minus)
plot(wantedMinus)"
    );
    let seen = live_bar(&source);
    assert_eq!(seen.len(), 4);
    let names = [
        "tr",
        "tr(true)",
        "atr",
        "kc basis",
        "kc upper",
        "kc lower",
        "supertrend",
        "direction",
        "plus",
        "minus",
    ];
    for (stage, plotted) in seen.iter().enumerate() {
        for (name, pair) in names.iter().zip(plotted.chunks(2)) {
            assert!(
                (pair[0] - pair[1]).abs() <= 1e-9 * pair[0].abs().max(pair[1].abs()).max(1.0),
                "stage {stage}: {name} {} against its formula {}",
                pair[0],
                pair[1]
            );
        }
    }
    // The changed tick moved the close, so an average of it moved too, and the close kept what the
    // last tick showed.
    assert_ne!(
        seen[1][6], seen[2][6],
        "the kc basis follows the changed tick"
    );
    assert_eq!(
        seen[2], seen[3],
        "the close folds what the last tick showed"
    );
}

#[test]
fn the_true_range_family_gives_the_same_values_on_a_shorter_chart() {
    // Replay runs the script on the bars so far: what it shows on a bar cannot depend on the
    // bars after it.
    let source = "//@version=6
indicator(\"Prefix\", overlay = true)
[line, direction] = ta.supertrend(3, 10)
[basis, upper, lower] = ta.kc(close, 10, 1.5)
[plus, minus, adx] = ta.dmi(14, 9)
plot(ta.tr)
plot(ta.atr(14))
plot(line)
plot(direction)
plot(basis)
plot(upper)
plot(lower)
plot(ta.kcw(close, 10, 1.5))
plot(plus)
plot(minus)
plot(adx)
";
    let whole = plots(source, 400);
    for count in [1, 2, 3, 15, 16, 120] {
        let part = plots(source, count);
        for (index, (whole, part)) in whole.iter().zip(&part).enumerate() {
            assert_same(
                &format!("plot {index} over {count} bars"),
                &whole[..count],
                part,
            );
        }
    }
}

/// The first eighty bars of BINANCE:BTCUSDT 5m (2026-11-02) with the default lengths, as TradingView's
/// Pine logs print them (`tests/data/tradingview-true-range-family.log`): `A` lines carry the bar and
/// what `ta.tr`, `ta.atr(14)`, `ta.supertrend(3, 10)`, `ta.kc(close, 20, 1.5)` and `ta.dmi(14, 9)`
/// gave on it; `B` lines what the same calls gave inside `if bar_index % 3 == 1`.
///
/// One column is not compared: a conditional `ta.supertrend`. TradingView's built-in prints `0` on
/// some of its first calls and then a band that is neither the reference code's nor the chart's
/// (the reference code run conditionally is what `a_conditional_supertrend_follows_the_reference_code`
/// pins); nothing in the documentation says what it is, so it is left as it was.
#[test]
fn the_true_range_family_matches_tradingview_over_eighty_bars() {
    use crate::model::Kline;
    let text = include_str!("data/tradingview-true-range-family.log");
    let parse = |t: &str| -> f64 {
        if t == "NaN" {
            f64::NAN
        } else {
            t.parse().unwrap()
        }
    };
    let mut data = Vec::new();
    let mut a_rows: Vec<Vec<f64>> = Vec::new();
    let mut b_rows: Vec<Vec<f64>> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts[0].starts_with('A') {
            let i = data.len() as i64;
            data.push(Kline {
                open_time: 1_000_000 + i * 300_000,
                close_time: 1_000_000 + (i + 1) * 300_000,
                open: parse(parts[1]),
                high: parse(parts[2]),
                low: parse(parts[3]),
                close: parse(parts[4]),
                volume: 100.0,
            });
            a_rows.push(parts[6..].iter().map(|t| parse(t)).collect());
        } else {
            b_rows.push(parts[1..].iter().map(|t| parse(t)).collect());
        }
    }
    let source = "//@version=6
indicator(\"p670b\", overlay = true)
[st, dir] = ta.supertrend(3, 10)
[kcm, kcu, kcl] = ta.kc(close, 20, 1.5)
[pdi, mdi, adx] = ta.dmi(14, 9)
atr14 = ta.atr(14)
trN = ta.tr
sometimes = bar_index % 3 == 1
float gAtr = na
float gSt = na
float gKcu = na
float gKcm = na
float gPdi = na
float gMdi = na
float gAdx = na
if sometimes
    gAtr := ta.atr(14)
    [s2, d2] = ta.supertrend(3, 10)
    gSt := s2
    [k1, k2, k3] = ta.kc(close, 20, 1.5)
    gKcm := k1
    gKcu := k2
    [p2, m2, a2] = ta.dmi(14, 9)
    gPdi := p2
    gMdi := m2
    gAdx := a2
plot(trN)
plot(atr14)
plot(st)
plot(dir)
plot(kcm)
plot(kcu)
plot(kcl)
plot(pdi)
plot(mdi)
plot(adx)
plot(gAtr)
plot(gSt)
plot(gKcm)
plot(gKcu)
plot(gPdi)
plot(gMdi)
plot(gAdx)
";
    let session = super::bench::run_script(source, &data, false, |_| {});
    let output = session.output();
    let a_names = [
        "tr", "atr", "st", "dir", "kcm", "kcu", "kcl", "pdi", "mdi", "adx",
    ];
    let b_names = ["gAtr", "gSt", "gKcm", "gKcu", "gPdi", "gMdi", "gAdx"];
    let mut mismatches = Vec::new();
    for (bar, (a, b)) in a_rows.iter().zip(&b_rows).enumerate() {
        let wanted: Vec<f64> = a.iter().chain(b).copied().collect();
        let names: Vec<&str> = a_names.iter().chain(&b_names).copied().collect();
        for (k, name) in names.iter().enumerate() {
            if *name == "gSt" {
                continue;
            }
            let got = output.plots[k].values[bar];
            let w = wanted[k];
            let ok = match got {
                None => w.is_nan(),
                Some(g) => (g - w).abs() <= 1e-6 * w.abs().max(1.0),
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {got:?} against TradingView's {w}"
                ));
            }
        }
    }
    assert_eq!(data.len(), 80);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_true_range_family_inside_a_request_measures_the_requested_bars() {
    // The previous close is the requested series' own, not the chart's: each built-in must give
    // what the reference code written out inside the same request gives.
    use super::bench::{bars, run_script};
    let source = format!(
        "//@version=6
indicator(\"Requested\", overlay = true)
{TRUE_RANGE_REFERENCE}{SUPERTREND_REFERENCE}{DMI_REFERENCE}
[kcBasis, kcUpper, kcLower] = request.security(syminfo.tickerid, \"5\", ta.kc(close, 20, 1.5))
[wantedPlus, wantedMinus, wantedAdx] = request.security(syminfo.tickerid, \"5\", f_dmi(14, 9))
[builtPlus, builtMinus, builtAdx] = request.security(syminfo.tickerid, \"5\", ta.dmi(14, 9))
[wantedLine, wantedDirection] = request.security(syminfo.tickerid, \"5\", f_supertrend(3, 10))
[builtLine, builtDirection] = request.security(syminfo.tickerid, \"5\", ta.supertrend(3, 10))
plot(request.security(syminfo.tickerid, \"5\", ta.tr))
plot(request.security(syminfo.tickerid, \"5\", f_tr(false)))
plot(request.security(syminfo.tickerid, \"5\", ta.atr(14)))
plot(request.security(syminfo.tickerid, \"5\", ta.rma(f_tr(true), 14)))
plot(kcBasis)
plot(request.security(syminfo.tickerid, \"5\", ta.ema(close, 20)))
plot(kcUpper)
plot(request.security(syminfo.tickerid, \"5\", ta.ema(close, 20) + 1.5 * ta.ema(f_tr(false), 20)))
plot(kcLower)
plot(request.security(syminfo.tickerid, \"5\", ta.ema(close, 20) - 1.5 * ta.ema(f_tr(false), 20)))
plot(builtPlus)
plot(wantedPlus)
plot(builtMinus)
plot(wantedMinus)
plot(builtAdx)
plot(wantedAdx)
plot(builtLine)
plot(wantedLine)
plot(builtDirection)
plot(wantedDirection)
"
    );
    let minutes = bars(900, 60_000);
    let plotted: Vec<Vec<Option<f64>>> = run_script(&source, &minutes, true, |_| {})
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect();
    let names = [
        "tr",
        "atr",
        "kc basis",
        "kc upper",
        "kc lower",
        "plus",
        "minus",
        "adx",
        "supertrend",
        "direction",
    ];
    for (name, pair) in names.iter().zip(plotted.chunks(2)) {
        assert_same(name, &pair[0], &pair[1]);
    }
    assert!(plotted[0].iter().any(Option::is_some), "the request ran");
}

/// The first two hundred bars of BINANCE:BTCUSDT 5m (2026-11-02) and what TradingView's Pine logs
/// print for the inputs the default-length probe did not use (`tests/data/tradingview-true-range-
/// inputs.log`): `A` lines are the bars, `C` lines the values on a bar (every one of the first forty,
/// then every tenth): `ta.tr`, `ta.atr(14)`, `ta.kc(close, 10, 2, false)` and `ta.kcw` of it,
/// `ta.kc(close, 20, 1.5, true)`, `ta.dmi(7, 20)`, `ta.dmi(14, 9)` and `ta.supertrend(3, 10)`.
#[test]
fn the_true_range_family_matches_tradingview_with_other_inputs_and_deeper_history() {
    use crate::model::Kline;
    let text = include_str!("data/tradingview-true-range-inputs.log");
    let parse = |t: &str| -> f64 {
        if t == "NaN" {
            f64::NAN
        } else {
            t.parse().expect("a number")
        }
    };
    let mut data = Vec::new();
    let mut rows: Vec<(usize, Vec<f64>)> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let bar: usize = parts[0][1..].parse().expect("a bar");
        if parts[0].starts_with('A') {
            let index = data.len() as i64;
            data.push(Kline {
                open_time: 1_000_000 + index * 300_000,
                close_time: 1_000_000 + (index + 1) * 300_000,
                open: parse(parts[1]),
                high: parse(parts[2]),
                low: parse(parts[3]),
                close: parse(parts[4]),
                volume: 100.0,
            });
        } else {
            rows.push((bar, parts[1..].iter().map(|t| parse(t)).collect()));
        }
    }
    let source = "//@version=6
indicator(\"Inputs\", overlay = true)
[kb, ku, kl] = ta.kc(close, 10, 2.0, false)
kw = ta.kcw(close, 10, 2.0, false)
[k2b, k2u, k2l] = ta.kc(close, 20, 1.5, true)
[p7, m7, a7] = ta.dmi(7, 20)
[p14, m14, a14] = ta.dmi(14, 9)
[st, dir] = ta.supertrend(3, 10)
plot(ta.tr)
plot(ta.atr(14))
plot(kb)
plot(ku)
plot(kl)
plot(kw)
plot(k2b)
plot(k2u)
plot(k2l)
plot(p7)
plot(m7)
plot(a7)
plot(p14)
plot(m14)
plot(a14)
plot(st)
plot(dir)
";
    let names = [
        "tr",
        "atr",
        "kc basis, high-low",
        "kc upper, high-low",
        "kc lower, high-low",
        "kcw, high-low",
        "kc basis",
        "kc upper",
        "kc lower",
        "plus(7, 20)",
        "minus(7, 20)",
        "adx(7, 20)",
        "plus",
        "minus",
        "adx",
        "supertrend",
        "direction",
    ];
    let session = super::bench::run_script(source, &data, false, |_| {});
    let output = session.output();
    assert_eq!(data.len(), 200);
    assert_eq!(rows.len(), 56);
    let mut mismatches = Vec::new();
    for (bar, wanted) in &rows {
        for ((name, wanted), plot) in names.iter().zip(wanted).zip(&output.plots) {
            let ok = match plot.values[*bar] {
                None => wanted.is_nan(),
                // TradingView prints eight decimals.
                Some(got) => (got - wanted).abs() <= 1e-9 * wanted.abs().max(1.0) + 6e-9,
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {:?} against TradingView's {wanted}",
                    plot.values[*bar]
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_true_range_family_follows_its_reference_code_over_flat_bars_and_gaps() {
    // Bars with no range at all (a halted market) and bars that open far from the last close: the
    // range is zero or all gap, so the averages and the directional index divide by little.
    use super::bench::{bars, run_script};
    let mut data = bars(400, 60_000);
    for index in 1..data.len() {
        let before = data[index - 1].close;
        if index % 7 == 0 {
            let bar = &mut data[index];
            (bar.open, bar.high, bar.low, bar.close) = (before, before, before, before);
        } else if index % 11 == 0 {
            let bar = &mut data[index];
            let shift = before * 1.04 - bar.open;
            bar.open += shift;
            bar.high += shift;
            bar.low += shift;
            bar.close += shift;
        }
    }
    let source = format!(
        "//@version=6
indicator(\"Flat and gapped\", overlay = true)
{TRUE_RANGE_REFERENCE}{SUPERTREND_REFERENCE}{DMI_REFERENCE}
[plus, minus, adx] = ta.dmi(10, 5)
[wantedPlus, wantedMinus, wantedAdx] = f_dmi(10, 5)
[line, direction] = ta.supertrend(2, 7)
[wantedLine, wantedDirection] = f_supertrend(2, 7)
[basis, upper, lower] = ta.kc(close, 8, 1.5)
plot(ta.tr)
plot(f_tr(false))
plot(ta.atr(7))
plot(ta.rma(f_tr(true), 7))
plot(plus)
plot(wantedPlus)
plot(minus)
plot(wantedMinus)
plot(adx)
plot(wantedAdx)
plot(line)
plot(wantedLine)
plot(direction)
plot(wantedDirection)
plot(upper)
plot(ta.ema(close, 8) + 1.5 * ta.ema(f_tr(false), 8))
plot(lower)
plot(ta.ema(close, 8) - 1.5 * ta.ema(f_tr(false), 8))
plot(ta.kcw(close, 8, 1.5))
plot(3 * ta.ema(f_tr(false), 8) / ta.ema(close, 8))
"
    );
    let plotted: Vec<Vec<Option<f64>>> = run_script(&source, &data, false, |_| {})
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect();
    let names = [
        "tr",
        "atr",
        "plus",
        "minus",
        "adx",
        "supertrend",
        "direction",
        "kc upper",
        "kc lower",
        "kcw",
    ];
    for (name, pair) in names.iter().zip(plotted.chunks(2)) {
        assert_same(name, &pair[0], &pair[1]);
    }
    assert!(
        plotted[8].iter().flatten().count() > 300,
        "the directional index ran through the flat bars"
    );
}

/// A close with a hole every 37th bar, the first included.
const HOLEY: &str = "src = bar_index % 37 == 0 ? na : close";

/// An exponential average written out from its definition for a source with holes: a hole gives
/// no value and leaves the state alone, the average starts as the mean of the first `length`
/// values that exist. `alpha` is `2 / (length + 1)` for an EMA and `1 / length` for an RMA.
fn skipping_average(name: &str, length: u32, alpha: &str) -> String {
    format!(
        "var float {name}Total = 0.0
var int {name}Seen = 0
var float {name}Last = na
float {name} = na
if not na(src)
    if na({name}Last)
        {name}Total += src
        {name}Seen += 1
        if {name}Seen == {length}
            {name}Last := {name}Total / {length}
            {name} := {name}Last
    else
        {name}Last := {alpha} * src + (1 - {alpha}) * {name}Last
        {name} := {name}Last
"
    )
}

#[test]
fn ema_and_rma_skip_a_missing_bar_and_keep_their_state() {
    let source = format!(
        "//@version=6
indicator(\"Skipping averages\")
{HOLEY}
{}{}plot(ta.ema(src, 20))
plot(ema)
plot(ta.rma(src, 14))
plot(rma)
",
        skipping_average("ema", 20, "(2.0 / 21)"),
        skipping_average("rma", 14, "(1.0 / 14)"),
    );
    let plotted = plots(&source, 900);
    assert_same("ema", &plotted[0], &plotted[1]);
    assert_same("rma", &plotted[2], &plotted[3]);
    assert_eq!(
        first_value(&plotted[0]),
        Some(20),
        "the 20th value that exists"
    );
    assert_eq!(plotted[0][37], None, "na on the na bar");
    assert!(plotted[0][38].is_some(), "and back on the next one");
    assert!(plotted[0].iter().flatten().count() > 800);
}

#[test]
fn the_averages_of_keltner_and_tsi_skip_a_missing_bar_too() {
    let source = format!(
        "//@version=6
indicator(\"Skipping legs\")
{HOLEY}
{}[basis, upper, lower] = ta.kc(src, 20, 1.5, false)
plot(basis)
plot(na(ema) or na(upper) ? na : ema)
plot(ta.tsi(src, 5, 13))
",
        skipping_average("ema", 20, "(2.0 / 21)"),
    );
    let plotted = plots(&source, 900);
    assert_same("kc basis", &plotted[0], &plotted[1]);
    assert_eq!(plotted[2][37], None, "tsi has no value on the na bar");
    assert!(plotted[2][60].is_some(), "and recovers after it");
}

#[test]
fn rsi_is_na_while_the_change_is_na_and_keeps_its_averages() {
    let source = format!(
        "//@version=6
indicator(\"Rsi\")
{HOLEY}
up = math.max(src - src[1], 0)
down = math.max(src[1] - src, 0)
{}{}
rs = upAverage / downAverage
plot(ta.rsi(src, 14))
plot(downAverage == 0 ? 100.0 : 100 - 100 / (1 + rs))
",
        // The same two skipping averages, named for the gain and the loss.
        skipping_average("upAverage", 14, "(1.0 / 14)").replace("src", "up"),
        skipping_average("downAverage", 14, "(1.0 / 14)").replace("src", "down"),
    );
    let plotted = plots(&source, 900);
    assert_same("rsi", &plotted[0], &plotted[1]);
    assert_eq!(plotted[0][37], None, "the na bar");
    assert_eq!(plotted[0][38], None, "the bar after it, whose change is na");
    assert!(plotted[0][39].is_some());
}

/// The last `length` values of a source with holes that exist, kept in an array.
const KEPT: &str = "var float[] kept = array.new_float()
if not na(src)
    kept.push(src)
    if kept.size() > 20
        kept.shift()
bool full = kept.size() == 20
";

#[test]
fn window_statistics_skip_a_missing_bar_and_hold_their_value() {
    let source = format!(
        "//@version=6
indicator(\"Windows\")
{HOLEY}
{KEPT}[basis, upper, lower] = ta.bb(src, 20, 2)
plot(ta.stdev(src, 20))
plot(full ? array.stdev(kept, true) : na)
plot(ta.variance(src, 20, false))
plot(full ? array.variance(kept, false) : na)
plot(ta.median(src, 20))
plot(full ? array.median(kept) : na)
plot(ta.range(src, 20))
plot(full ? array.max(kept) - array.min(kept) : na)
plot(math.sum(src, 20))
plot(full ? array.sum(kept) : na)
plot(upper)
plot(full ? array.avg(kept) + 2 * array.stdev(kept, true) : na)
plot(ta.bbw(src, 20, 2))
plot(full ? 4 * array.stdev(kept, true) / array.avg(kept) * 100 : na)
",
    );
    let plotted = plots(&source, 900);
    for (label, index) in [
        ("stdev", 0),
        ("variance", 2),
        ("median", 4),
        ("range", 6),
        ("sum", 8),
        ("bb upper", 10),
        ("bbw", 12),
    ] {
        assert_same(label, &plotted[index], &plotted[index + 1]);
        assert!(plotted[index][37].is_some(), "{label}: held on the na bar");
        assert_eq!(plotted[index][37], plotted[index][36], "{label}: held");
    }
}

#[test]
fn wma_is_na_on_the_missing_bar_and_fills_its_slot_afterwards() {
    let source = format!(
        "//@version=6
indicator(\"Wma\")
{HOLEY}
var float lastGood = na
var float[] slots = array.new_float()
slots.push(na(src) ? lastGood : src)
if slots.size() > 20
    slots.shift()
lastGood := na(src) ? lastGood : src
float weighted = 0.0
for index = 0 to slots.size() - 1
    weighted += (index + 1) * slots.get(index)
plot(ta.wma(src, 20))
plot(na(src) or slots.size() < 20 ? na : weighted / 210)
",
    );
    let plotted = plots(&source, 900);
    assert_same("wma", &plotted[0], &plotted[1]);
    assert_eq!(plotted[0][37], None);
    assert!(plotted[0][38].is_some(), "the slot is filled, not a gap");
}

#[test]
fn dev_stays_strict_like_on_tradingview() {
    let source = format!(
        "//@version=6
indicator(\"Dev\")
{HOLEY}
plot(ta.dev(src, 20))
plot(ta.barssince(na(src)))
"
    );
    let plotted = plots(&source, 400);
    for (bar, (dev, since)) in plotted[0].iter().zip(&plotted[1]).enumerate().skip(20) {
        let since = since.expect("a hole exists from bar 0");
        assert_eq!(
            dev.is_none(),
            since < 20.0,
            "bar {bar}: a hole {since} bars back"
        );
    }
}

/// A number as TradingView's logs print it: `NaN` is `na`.
fn logged_number(text: &str) -> f64 {
    if text == "NaN" {
        f64::NAN
    } else {
        text.parse().expect("a number")
    }
}

/// The `A` lines of a TradingView log as bars (open, high, low, close, volume), five minutes apart.
fn logged_bars(text: &str) -> Vec<crate::model::Kline> {
    text.lines()
        .filter(|line| line.starts_with('A'))
        .enumerate()
        .map(|(index, line)| {
            let parts: Vec<f64> = line.split_whitespace().skip(1).map(logged_number).collect();
            crate::model::Kline {
                open_time: 1_000_000 + index as i64 * 300_000,
                close_time: 1_000_000 + (index as i64 + 1) * 300_000,
                open: parts[0],
                high: parts[1],
                low: parts[2],
                close: parts[3],
                volume: parts[4],
            }
        })
        .collect()
}

/// The lines of a TradingView log that start with `tag`: the bar and its values.
fn logged_rows(text: &str, tag: char) -> Vec<(usize, Vec<f64>)> {
    text.lines()
        .filter(|line| line.starts_with(tag))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let bar = parts.next().expect("a bar")[1..].parse().expect("a bar");
            (bar, parts.map(logged_number).collect())
        })
        .collect()
}

/// TradingView's own output (BINANCE:BTCUSDT 5m, the first 120 bars from 2026-11-02 00:00) for a
/// source with a missing value every 37th bar, the first included: `src = bar_index % 37 == 0 ? na :
/// close`. `A` lines are the bars (open, high, low, close, volume), `C` lines the 42 results the
/// probe logged on each, in the order of `MISSING_BAR_COLUMNS`; `NaN` is `na`. The values this change
/// rewrote (`ta.ema`, `ta.rma`, `ta.rsi`, the window statistics, `ta.wma`, `ta.macd`, `ta.kc`,
/// `ta.tsi`) match on all 120 bars.
#[test]
fn a_missing_source_bar_gives_what_tradingview_gives() {
    let text = include_str!("data/tradingview-missing-bars.log");
    let data = logged_bars(text);
    let rows = logged_rows(text, 'C');
    let columns: Vec<(&str, &str)> = MISSING_BAR_COLUMNS.to_vec();
    let mut source = String::from("//@version=6\nindicator(\"Missing\", overlay = true)\n");
    source.push_str("src = bar_index % 37 == 0 ? na : close\n");
    source.push_str("[bbm, bbu, bbl] = ta.bb(src, 20, 2)\n");
    source.push_str("[ml, ms, mh] = ta.macd(src, 12, 26, 9)\n");
    source.push_str("[kcb, kcu, kcl] = ta.kc(src, 20, 1.5, true)\n");
    for (_, code) in &columns {
        source.push_str(&format!("plot({code})\n"));
    }
    let session = super::bench::run_script(&source, &data, false, |_| {});
    let output = session.output();
    assert_eq!(output.plots.len(), columns.len());
    let mut mismatches = Vec::new();
    for (bar, wanted) in &rows {
        for (((name, _), wanted), plot) in columns.iter().zip(wanted).zip(&output.plots) {
            if NOT_YET_LIKE_TRADINGVIEW.contains(name) {
                continue;
            }
            let ok = match plot.values[*bar] {
                None => wanted.is_nan(),
                // TradingView prints eight decimals.
                Some(got) => (got - wanted).abs() <= 1e-8 * wanted.abs().max(1.0) + 6e-9,
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {:?} against TradingView's {wanted}",
                    plot.values[*bar]
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} values differ, first: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(8)]
    );
}

/// The columns of `MISSING_BAR_COLUMNS` that still differ from TradingView over a missing bar, left
/// out of the comparison above. Each is a function the change that added this test did not touch;
/// what TradingView does is in `docs/pine-ta-parity.md`.
const NOT_YET_LIKE_TRADINGVIEW: [&str; 14] = [
    "ta.cmo(10)",
    "ta.correlation(10)",
    "ta.hma(9)",
    "ta.mfi(14)",
    "ta.mode(10)",
    "ta.vwma(10)",
    "ta.percentrank(10)",
    "ta.highest(10)",
    "ta.lowest(10)",
    "ta.highestbars(10)",
    "ta.lowestbars(10)",
    "ta.rising(3)",
    "ta.falling(3)",
    "ta.cum",
];

/// What the probe logged on each bar, in order: the column's name and the Pine that draws it.
const MISSING_BAR_COLUMNS: [(&str, &str); 42] = [
    ("ta.ema(10)", "ta.ema(src, 10)"),
    ("ta.rma(10)", "ta.rma(src, 10)"),
    ("ta.rsi(14)", "ta.rsi(src, 14)"),
    ("ta.stdev(20)", "ta.stdev(src, 20)"),
    ("ta.variance(20)", "ta.variance(src, 20)"),
    ("ta.median(20)", "ta.median(src, 20)"),
    ("ta.range(20)", "ta.range(src, 20)"),
    ("math.sum(20)", "math.sum(src, 20)"),
    ("ta.wma(10)", "ta.wma(src, 10)"),
    ("ta.dev(20)", "ta.dev(src, 20)"),
    ("ta.bb upper", "bbu"),
    ("ta.bbw(20, 2)", "ta.bbw(src, 20, 2)"),
    ("ta.macd line", "ml"),
    ("ta.macd signal", "ms"),
    ("ta.macd histogram", "mh"),
    ("ta.kc basis", "kcb"),
    ("ta.kc upper", "kcu"),
    ("ta.tsi(5, 13)", "ta.tsi(src, 5, 13)"),
    ("ta.cmo(10)", "ta.cmo(src, 10)"),
    ("ta.cci(20)", "ta.cci(src, 20)"),
    ("ta.hma(9)", "ta.hma(src, 9)"),
    ("ta.cog(10)", "ta.cog(src, 10)"),
    ("ta.correlation(10)", "ta.correlation(src, open, 10)"),
    ("ta.mfi(14)", "ta.mfi(src, 14)"),
    ("ta.vwma(10)", "ta.vwma(src, 10)"),
    ("ta.wpr(14)", "ta.wpr(14)"),
    ("ta.highestbars(10)", "ta.highestbars(src, 10)"),
    ("ta.lowestbars(10)", "ta.lowestbars(src, 10)"),
    ("ta.rising(3)", "ta.rising(src, 3) ? 1 : 0"),
    ("ta.falling(3)", "ta.falling(src, 3) ? 1 : 0"),
    ("ta.mode(10)", "ta.mode(src, 10)"),
    ("ta.percentrank(10)", "ta.percentrank(src, 10)"),
    ("ta.linreg(10, 0)", "ta.linreg(src, 10, 0)"),
    ("ta.mom(5)", "ta.mom(src, 5)"),
    ("ta.roc(5)", "ta.roc(src, 5)"),
    ("ta.change", "ta.change(src)"),
    ("ta.sma(10)", "ta.sma(src, 10)"),
    ("ta.highest(10)", "ta.highest(src, 10)"),
    ("ta.lowest(10)", "ta.lowest(src, 10)"),
    ("ta.swma", "ta.swma(src)"),
    ("ta.alma(9, 0.85, 6)", "ta.alma(src, 9, 0.85, 6)"),
    ("ta.cum", "ta.cum(src)"),
];

#[test]
fn an_average_over_a_missing_bar_follows_a_live_bar_like_its_reference_code() {
    // Bar 1 has no source value. The live bar is recomputed on every tick from the averages as the
    // history left them and folded in only when it closes: `ta.ema` and `ta.rma` must give what the
    // formula written out in Pine gives at the history, the first tick, a changed tick and the close.
    let source = "src = bar_index == 1 ? na : close
f_average(x, length, alpha) =>
    var float seedSum = 0.0
    var int seen = 0
    var float prior = na
    float result = na
    if not na(x)
        if not na(prior)
            result := alpha * x + (1.0 - alpha) * prior
            prior := result
        else
            seedSum += x
            seen += 1
            if seen == length
                result := seedSum / length
                prior := result
    result
plot(ta.ema(src, 2))
plot(f_average(src, 2, 2.0 / 3.0))
plot(ta.rma(src, 2))
plot(f_average(src, 2, 0.5))";
    let seen = live_bar(source);
    assert_eq!(seen.len(), 4);
    for (stage, plotted) in seen.iter().enumerate() {
        for (name, pair) in ["ta.ema", "ta.rma"].iter().zip(plotted.chunks(2)) {
            assert!(
                (pair[0] - pair[1]).abs() <= 1e-9 * pair[0].abs().max(1.0),
                "stage {stage}: {name} {} against its formula {}",
                pair[0],
                pair[1]
            );
        }
    }
    assert_ne!(seen[1][0], seen[2][0], "the changed tick moved the average");
    assert_eq!(
        seen[2], seen[3],
        "the close folds what the last tick showed"
    );
}

/// A close with four holes in a row (bars 10 to 13 of every 40).
const HOLE_RUNS: &str = "src = bar_index % 40 >= 10 and bar_index % 40 < 14 ? na : close";

#[test]
fn a_run_of_missing_bars_is_skipped_like_a_single_one() {
    // The averages keep their state through the run, and each window statistic is taken over the
    // last values that exist and holds its value on every missing bar. Checked against the
    // definitions written out in Pine; TradingView was measured on single holes only.
    let source = format!(
        "//@version=6
indicator(\"Hole runs\")
{HOLE_RUNS}
{}{}var float[] valid = array.new_float()
if not na(src)
    valid.push(src)
    if valid.size() > 20
        valid.shift()
var float lastGood = na
var float[] slots = array.new_float()
slots.push(na(src) ? lastGood : src)
if slots.size() > 10
    slots.shift()
lastGood := na(src) ? lastGood : src
float weighted = 0.0
for index = 0 to slots.size() - 1
    weighted += (index + 1) * slots.get(index)
bool full = valid.size() == 20
plot(ta.ema(src, 20))
plot(ema)
plot(ta.rma(src, 14))
plot(rma)
plot(ta.stdev(src, 20))
plot(full ? valid.stdev() : na)
plot(ta.variance(src, 20))
plot(full ? valid.variance() : na)
plot(ta.median(src, 20))
plot(full ? valid.median() : na)
plot(ta.range(src, 20))
plot(full ? valid.range() : na)
plot(math.sum(src, 20))
plot(full ? valid.sum() : na)
plot(ta.wma(src, 10))
plot(na(src) or slots.size() < 10 ? na : weighted / 55)
",
        skipping_average("ema", 20, "(2.0 / 21)"),
        skipping_average("rma", 14, "(1.0 / 14)"),
    );
    let plotted = plots(&source, 400);
    for (index, name) in [
        "ema", "rma", "stdev", "variance", "median", "range", "sum", "wma",
    ]
    .iter()
    .enumerate()
    {
        assert_same(name, &plotted[index * 2], &plotted[index * 2 + 1]);
    }
    // The statistics hold their value through the whole run; the average and the wma are na on it.
    assert_eq!(
        plotted[4][49], plotted[4][50],
        "stdev holds on a missing bar"
    );
    assert_eq!(plotted[4][52], plotted[4][49], "and through the run");
    assert!(plotted[4][53].is_some());
    assert_eq!(plotted[0][51], None, "the average is na on a missing bar");
    assert!(plotted[0][54].is_some(), "and back after the run");
    assert_eq!(plotted[14][51], None);
    assert!(
        plotted[14][54].is_some(),
        "the wma recovers on the bar after"
    );
}

/// TradingView's output for four missing bars in a row (bars 10 to 13 of every 40) on the same 100
/// bars as above: `src = bar_index % 40 >= 10 and bar_index % 40 < 14 ? na : close`. `D` lines hold
/// the 18 results of the functions this change rewrote.
#[test]
fn a_run_of_missing_bars_gives_what_tradingview_gives() {
    let data = logged_bars(include_str!("data/tradingview-missing-bars.log"));
    let rows = logged_rows(include_str!("data/tradingview-missing-bar-runs.log"), 'D');
    assert_eq!(rows.len(), 100);
    let columns = [
        ("ta.ema(10)", "ta.ema(src, 10)"),
        ("ta.rma(10)", "ta.rma(src, 10)"),
        ("ta.rsi(14)", "ta.rsi(src, 14)"),
        ("ta.stdev(20)", "ta.stdev(src, 20)"),
        ("ta.variance(20)", "ta.variance(src, 20)"),
        ("ta.median(20)", "ta.median(src, 20)"),
        ("ta.range(20)", "ta.range(src, 20)"),
        ("math.sum(20)", "math.sum(src, 20)"),
        ("ta.wma(10)", "ta.wma(src, 10)"),
        ("ta.dev(20)", "ta.dev(src, 20)"),
        ("ta.bb upper", "bbu"),
        ("ta.bbw(20, 2)", "ta.bbw(src, 20, 2)"),
        ("ta.macd line", "ml"),
        ("ta.macd signal", "ms"),
        ("ta.macd histogram", "mh"),
        ("ta.kc basis", "kcb"),
        ("ta.kc upper", "kcu"),
        ("ta.tsi(5, 13)", "ta.tsi(src, 5, 13)"),
    ];
    let mut source = String::from("//@version=6\nindicator(\"Runs\", overlay = true)\n");
    source.push_str("src = bar_index % 40 >= 10 and bar_index % 40 < 14 ? na : close\n");
    source.push_str("[bbm, bbu, bbl] = ta.bb(src, 20, 2)\n");
    source.push_str("[ml, ms, mh] = ta.macd(src, 12, 26, 9)\n");
    source.push_str("[kcb, kcu, kcl] = ta.kc(src, 20, 1.5, true)\n");
    for (_, code) in &columns {
        source.push_str(&format!("plot({code})\n"));
    }
    let session = super::bench::run_script(&source, &data[..100], false, |_| {});
    let output = session.output();
    assert_eq!(output.plots.len(), columns.len());
    let mut mismatches = Vec::new();
    for (bar, wanted) in &rows {
        for (((name, _), wanted), plot) in columns.iter().zip(wanted).zip(&output.plots) {
            let ok = match plot.values[*bar] {
                None => wanted.is_nan(),
                Some(got) => (got - wanted).abs() <= 1e-8 * wanted.abs().max(1.0) + 6e-9,
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {:?} against TradingView's {wanted}",
                    plot.values[*bar]
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} values differ, first: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn what_a_missing_bar_gives_does_not_depend_on_the_bars_after_it() {
    // Replay runs the script on the bars so far: a bar must show the same value whether or not later
    // bars exist, holes included.
    let source = format!(
        "//@version=6
indicator(\"Prefix\", overlay = true)
{HOLEY}
[bbm, bbu, bbl] = ta.bb(src, 20, 2)
[ml, ms, mh] = ta.macd(src, 12, 26, 9)
[kcb, kcu, kcl] = ta.kc(src, 20, 1.5, true)
plot(ta.ema(src, 10))
plot(ta.rma(src, 10))
plot(ta.rsi(src, 14))
plot(ta.stdev(src, 20))
plot(ta.variance(src, 20))
plot(ta.median(src, 20))
plot(ta.range(src, 20))
plot(math.sum(src, 20))
plot(ta.wma(src, 10))
plot(ta.dev(src, 20))
plot(bbu)
plot(ta.bbw(src, 20, 2))
plot(ml)
plot(ms)
plot(mh)
plot(kcb)
plot(kcu)
plot(ta.tsi(src, 5, 13))
"
    );
    let whole = plots(&source, 300);
    // Counts that end on a hole (37, 74), just after one, and inside the first windows.
    for count in [1, 2, 10, 20, 37, 38, 74, 75, 150] {
        let part = plots(&source, count);
        for (index, (whole, part)) in whole.iter().zip(&part).enumerate() {
            assert_same(
                &format!("plot {index} over {count} bars"),
                &whole[..count],
                part,
            );
        }
    }
}

#[test]
fn odd_lengths_and_sources_give_na_or_the_obvious_value_and_never_stop_the_script() {
    let source = format!(
        "//@version=6
indicator(\"Odd\", overlay = true)
{HOLEY}
float none = na
float lastHole = barstate.islast ? na : close
plot(src)
plot(ta.ema(src, 1))
plot(ta.rma(src, 1))
plot(ta.stdev(src, 1))
plot(ta.wma(src, 1))
plot(ta.ema(src, 500))
plot(ta.stdev(src, 500))
plot(ta.rsi(src, 500))
plot(ta.ema(none, 5))
plot(ta.rsi(none, 14))
plot(ta.stdev(none, 5))
plot(ta.wma(none, 5))
plot(ta.median(none, 5))
plot(ta.ema(lastHole, 5))
plot(ta.stdev(lastHole, 5))
plot(ta.rsi(lastHole, 5))
plot(ta.ema(close, 5))
"
    );
    let plotted = plots(&source, 100);
    assert_eq!(plotted.len(), 17, "the script ran to the end");
    // Length 1: alpha is 1, so the average is the source; a window of one value has no spread.
    assert_same("ema(1)", &plotted[1], &plotted[0]);
    assert_same("rma(1)", &plotted[2], &plotted[0]);
    assert_same("wma(1)", &plotted[4], &plotted[0]);
    assert_eq!(plotted[3][0], None, "bar 0 is a hole");
    assert!(
        plotted[3][1..].iter().all(|value| *value == Some(0.0)),
        "stdev of one value"
    );
    // A length longer than the chart, or a source that never has a value: no value, no error.
    for index in [5, 6, 7, 8, 9, 10, 11, 12] {
        assert!(
            plotted[index].iter().all(Option::is_none),
            "plot {index} has no value"
        );
    }
    // A hole on the last bar only: the average is na there and was the ordinary one before it, the
    // statistic holds its value, rsi has no change.
    assert_same(
        "ema before the last bar",
        &plotted[13][..99],
        &plotted[16][..99],
    );
    assert_eq!(plotted[13][99], None);
    assert_eq!(plotted[14][99], plotted[14][98]);
    assert!(plotted[14][99].is_some());
    assert_eq!(plotted[15][99], None);
    assert!(plotted[15][98].is_some());
}

#[test]
fn averages_and_statistics_inside_a_request_skip_a_missing_bar_like_their_reference_code() {
    // The requested series has its own history and its own holes: each built-in must give what the
    // definition written out inside the same request gives.
    let source = "//@version=6
indicator(\"Requested\", overlay = true)
f_average(x, length, alpha) =>
    var float seedSum = 0.0
    var int seen = 0
    var float prior = na
    float result = na
    if not na(x)
        if not na(prior)
            result := alpha * x + (1.0 - alpha) * prior
            prior := result
        else
            seedSum += x
            seen += 1
            if seen == length
                result := seedSum / length
                prior := result
    result
f_stdev(x, length) =>
    var float[] valid = array.new_float()
    if not na(x)
        valid.push(x)
        if valid.size() > length
            valid.shift()
    valid.size() == length ? valid.stdev() : na
plot(request.security(syminfo.tickerid, \"5\", ta.ema(bar_index % 7 == 3 ? na : close, 20)))
plot(request.security(syminfo.tickerid, \"5\", f_average(bar_index % 7 == 3 ? na : close, 20, 2.0 / 21)))
plot(request.security(syminfo.tickerid, \"5\", ta.rma(bar_index % 7 == 3 ? na : close, 14)))
plot(request.security(syminfo.tickerid, \"5\", f_average(bar_index % 7 == 3 ? na : close, 14, 1.0 / 14)))
plot(request.security(syminfo.tickerid, \"5\", ta.stdev(bar_index % 7 == 3 ? na : close, 20)))
plot(request.security(syminfo.tickerid, \"5\", f_stdev(bar_index % 7 == 3 ? na : close, 20)))
";
    let minutes = bars(900, 60_000);
    let plotted: Vec<Vec<Option<f64>>> = run_script(source, &minutes, true, |_| {})
        .output()
        .plots
        .into_iter()
        .map(|plot| plot.values)
        .collect();
    for (name, pair) in ["ema", "rma", "stdev"].iter().zip(plotted.chunks(2)) {
        assert_same(name, &pair[0], &pair[1]);
    }
    assert!(plotted[0].iter().flatten().count() > 100, "the request ran");
}

/// TradingView's output when the functions are called only on every third bar (`if bar_index % 3 ==
/// 1`) over a source with a hole every tenth bar (`bar_index % 10 == 4`), on the same bars as above:
/// `E` lines hold what each gave on the bars it ran on (`tests/data/tradingview-missing-bars-
/// conditional.log`). Every column matches but `ta.wma`, which is not compared: TradingView's
/// conditional `ta.wma` is `na` on the first 5 calls and then a value that is neither the last five
/// valid calls', nor the chart bars' (hole filled or skipped), nor the unconditional rule's (see
/// `docs/pine-ta-parity.md`).
#[test]
fn conditional_calls_over_a_missing_bar_match_tradingview() {
    let data = logged_bars(include_str!("data/tradingview-missing-bars.log"));
    let rows = logged_rows(
        include_str!("data/tradingview-missing-bars-conditional.log"),
        'E',
    );
    assert_eq!(rows.len(), 33);
    let columns = [
        ("ta.ema(5)", "a1"),
        ("ta.rma(5)", "a2"),
        ("ta.rsi(5)", "a3"),
        ("ta.stdev(5)", "a4"),
        ("ta.variance(5)", "a5"),
        ("ta.median(5)", "a6"),
        ("ta.range(5)", "a7"),
        ("math.sum(5)", "a8"),
        ("ta.wma(5)", "a9"),
        ("ta.bb upper", "bu"),
        ("ta.macd line", "ml"),
        ("ta.macd signal", "ms"),
        ("ta.tsi(2, 3)", "t1"),
    ];
    let mut source = String::from("//@version=6\nindicator(\"Conditional\", overlay = true)\n");
    source.push_str("src = bar_index % 10 == 4 ? na : close\n");
    for (_, name) in &columns {
        source.push_str(&format!("float {name} = na\n"));
    }
    source.push_str(
        "if bar_index % 3 == 1
    a1 := ta.ema(src, 5)
    a2 := ta.rma(src, 5)
    a3 := ta.rsi(src, 5)
    a4 := ta.stdev(src, 5)
    a5 := ta.variance(src, 5)
    a6 := ta.median(src, 5)
    a7 := ta.range(src, 5)
    a8 := math.sum(src, 5)
    a9 := ta.wma(src, 5)
    [bm, bbu, bl] = ta.bb(src, 5, 2)
    bu := bbu
    [line, signal, hist] = ta.macd(src, 3, 5, 2)
    ml := line
    ms := signal
    t1 := ta.tsi(src, 2, 3)
",
    );
    for (_, code) in &columns {
        source.push_str(&format!("plot({code})\n"));
    }
    let session = super::bench::run_script(&source, &data[..100], false, |_| {});
    let output = session.output();
    let mut mismatches = Vec::new();
    for (bar, wanted) in &rows {
        for (((name, _), wanted), plot) in columns.iter().zip(wanted).zip(&output.plots) {
            if *name == "ta.wma(5)" {
                continue;
            }
            let ok = match plot.values[*bar] {
                None => wanted.is_nan(),
                Some(got) => (got - wanted).abs() <= 1e-8 * wanted.abs().max(1.0) + 6e-9,
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {:?} against TradingView's {wanted}",
                    plot.values[*bar]
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} values differ, first: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(8)]
    );
}
