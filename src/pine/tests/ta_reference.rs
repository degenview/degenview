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
plot(na(range) ? na : average)
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
