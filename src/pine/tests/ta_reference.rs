//! A `ta.*` built-in against the formula TradingView documents for it, written out in Pine from the
//! definition. Each test plots both and compares them bar by bar (the same bars, `na` where the
//! other is `na`), on a random walk, so a built-in that drifts from its definition fails here.

use super::bench::{bars, run_script};

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
