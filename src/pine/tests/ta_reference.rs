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
