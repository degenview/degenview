//! `ta.*` indicators over the per-call-site history the session keeps, ported from
//! `Builtins/PineTA.swift`. Pure: everything an indicator needs is in `State`.
//!
//! The session calls `evaluate` once per bar per call site. The state remembers every source value
//! (`inputs`, `na` included) and every result (`results`), so windows are read off the history and
//! the recursive averages (`ema`, `rma`) continue from the last result. Faithful quirks kept on
//! purpose: `ta.variance` and `ta.stdev` are population statistics, and a window with any `na` yields
//! `na`.

pub mod call;
pub mod extra;

use crate::model::Kline;
use crate::pine::runtime::value::RuntimeValue;
use extra::derived;

/// What one call site remembers between bars.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// One entry per evaluation. Usually the indicator's value; for `ta.cross*` the
    /// `[source, second]` pair.
    pub results: Vec<RuntimeValue>,
    /// The source series, one entry per evaluation, `na` included.
    pub inputs: Vec<RuntimeValue>,
    /// The bar's high, low and close, one entry per evaluation — the history `ta.sar` reads
    /// (`ta.dmi` keeps the previous high and low here).
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub closes: Vec<f64>,
    /// The second series of a two-source call (`ta.correlation`), one entry per evaluation.
    pub seconds: Vec<RuntimeValue>,
    /// `ta.sar` state: `[sar, extreme point, acceleration, 1 when the SAR is below price]`.
    pub sar: Option<[f64; 4]>,
    /// `ta.supertrend` state.
    pub supertrend: Option<Supertrend>,
    /// Series a function derives from its inputs, with their own moving averages (`ta.kc`, `ta.tsi`).
    pub derived: Vec<extra::Derived>,
    /// `ta.pivot_point_levels` state: the period since the last anchor and the levels it last set.
    pub pivot: Option<extra::PivotPeriod>,
    /// `ta.dmi`'s four smoothed series, each the state of its own `ta.rma`. Boxed: `State` is cloned on every realtime tick, and
    /// only a `ta.dmi` site has smoothers.
    pub dmi: Option<Box<[State; 4]>>,
    /// `timeframe.change`: the open time of the bar the site last ran on, and of the bar it ran on
    /// before that. Boxed: `State` is cloned on every realtime tick, and only a `timeframe.change`
    /// site has bars.
    pub bar_times: Option<Box<BarTimes>>,
}

/// The bars a call site has run on, newest first. Only a bar the site ran on is recorded, so a site
/// under an `if` or a short-circuit sees the last bar it ran on, not the chart's previous bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarTimes {
    pub latest: i64,
    pub before: Option<i64>,
}

/// `ta.supertrend` between bars: the final upper and lower bands and the line of the bar before.
/// `NaN` stands for `na`, so a comparison with a missing value is false as it is in Pine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Supertrend {
    pub upper: f64,
    pub lower: f64,
    pub line: f64,
}

/// `ta.macd`'s lengths when a call leaves them out (TradingView requires all three; the session
/// binds them in `inline_call`).
pub const MACD_DEFAULTS: [i64; 3] = [12, 26, 9];

/// The `ta.*` names `evaluate` computes from one source series and a length. The bar-relative ones
/// (`ta.sar`, `ta.supertrend`) go through `evaluate_price` instead.
pub const SUPPORTED: [&str; 27] = [
    "ta.sma",
    "ta.ema",
    "ta.rma",
    "ta.highest",
    "ta.lowest",
    "ta.change",
    "ta.wma",
    "ta.rising",
    "ta.falling",
    "ta.mom",
    "ta.roc",
    "ta.rsi",
    "ta.cross",
    "ta.crossover",
    "ta.crossunder",
    "ta.median",
    "ta.range",
    "ta.dev",
    "ta.swma",
    "ta.cmo",
    "ta.cci",
    "ta.hma",
    "ta.highestbars",
    "ta.lowestbars",
    "ta.percentrank",
    "ta.max",
    "ta.min",
];

pub fn is_supported(name: &str) -> bool {
    SUPPORTED.contains(&name) || matches!(name, "ta.linreg" | "math.sum")
}

fn float(value: f64) -> RuntimeValue {
    RuntimeValue::Float(value)
}

fn optional(value: Option<f64>) -> RuntimeValue {
    value.map_or(RuntimeValue::Na, float)
}

/// Evaluates `name` for the current bar and records the bar in `state`.
pub fn evaluate(
    name: &str,
    source: &RuntimeValue,
    second: &RuntimeValue,
    length: i64,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    // A length that is not a positive count reads as no window (most indicators then yield `na`).
    let count = usize::try_from(length).unwrap_or(0);
    let inputs = &state.inputs;
    let result = match name {
        "ta.sma" => sma(inputs, count),
        "ta.ema" => optional(derived(state, 0).ema(source.number(), count)),
        "ta.rma" => optional(derived(state, 0).rma(source.number(), count)),
        "ta.highest" => extremum(inputs, count, f64::max),
        "ta.lowest" => extremum(inputs, count, f64::min),
        "ta.change" => change(inputs, if count > 0 { count } else { 1 }),
        "ta.wma" => wma(inputs, count),
        "ta.rising" => trend(inputs, count, true),
        "ta.falling" => trend(inputs, count, false),
        "ta.mom" | "ta.roc" => momentum(inputs, count, name == "ta.roc"),
        "ta.rsi" => extra::rsi(&mut *state, length),
        "ta.median" => skipping_statistic(inputs, count, median),
        "ta.range" => skipping_statistic(inputs, count, |values| {
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            if values.is_empty() { 0.0 } else { high - low }
        }),
        "ta.dev" => statistic(inputs, count, mean_deviation),
        "ta.swma" => statistic(inputs, 4, |v| (v[0] + 2.0 * v[1] + 2.0 * v[2] + v[3]) / 6.0),
        "ta.cmo" => cmo(inputs, count),
        "ta.cci" => cci(inputs, count, state.results.last()),
        "ta.hma" => hma(inputs, count),
        "ta.highestbars" => extremum_bars(inputs, count, true),
        "ta.lowestbars" => extremum_bars(inputs, count, false),
        "ta.percentrank" => percent_rank(inputs, count),
        "ta.linreg" => linreg(inputs, count, second.int_value().unwrap_or(0)),
        "math.sum" => skipping_statistic(inputs, count, |values| values.iter().sum()),
        "ta.max" => running_extreme(source, state.results.last(), f64::max),
        "ta.min" => running_extreme(source, state.results.last(), f64::min),
        "ta.cross" | "ta.crossover" | "ta.crossunder" => {
            let crossed = cross(name, source, second, state.results.last());
            state
                .results
                .push(RuntimeValue::Tuple(vec![source.clone(), second.clone()]));
            return crossed;
        }
        _ => RuntimeValue::Na,
    };
    state.results.push(result.clone());
    result
}

/// Evaluates a `ta.*` that reads the bar's own high/low/close rather than a source series
/// (`ta.sar`, `ta.supertrend`). The session calls this once per bar per call site;
/// `previous_close` is the chart's `close[1]`, which `ta.supertrend` measures against.
pub fn evaluate_price(
    name: &str,
    arguments: &[RuntimeValue],
    bar: &Kline,
    previous_close: Option<f64>,
    state: &mut State,
) -> RuntimeValue {
    match name {
        "ta.sar" => {
            let start = arguments
                .first()
                .and_then(RuntimeValue::number)
                .unwrap_or(0.02);
            let inc = arguments
                .get(1)
                .and_then(RuntimeValue::number)
                .unwrap_or(0.02);
            let max = arguments
                .get(2)
                .and_then(RuntimeValue::number)
                .unwrap_or(0.2);
            evaluate_sar(state, bar, start, inc, max)
        }
        "ta.supertrend" => {
            let factor = arguments
                .first()
                .and_then(RuntimeValue::number)
                .unwrap_or(3.0);
            let period = arguments
                .get(1)
                .and_then(RuntimeValue::number)
                .unwrap_or(10.0);
            evaluate_supertrend(state, bar, previous_close, factor, period)
        }
        _ => RuntimeValue::Na,
    }
}

/// `ta.sar(start, inc, max)`: Wilder's parabolic SAR, following the Pine reference. The state is
/// `[sar, extreme point, acceleration, 1 when the SAR is below price]`.
fn evaluate_sar(state: &mut State, bar: &Kline, start: f64, inc: f64, max: f64) -> RuntimeValue {
    let index = state.highs.len();
    let last_high = state.highs.last().copied();
    let last_low = state.lows.last().copied();
    let last_close = state.closes.last().copied();
    let older_high = index.checked_sub(2).map(|i| state.highs[i]);
    let older_low = index.checked_sub(2).map(|i| state.lows[i]);

    let value = if index == 1 {
        let (Some(last_high), Some(last_low), Some(last_close)) = (last_high, last_low, last_close)
        else {
            state.highs.push(bar.high);
            state.lows.push(bar.low);
            state.closes.push(bar.close);
            return RuntimeValue::Na;
        };
        let below = bar.close > last_close;
        let mut sar = [
            if below { last_low } else { last_high },
            if below { bar.high } else { bar.low },
            start,
            if below { 1.0 } else { 0.0 },
        ];
        advance_sar(
            &mut sar, bar, start, inc, max, last_high, last_low, older_high, older_low, index,
        );
        state.sar = Some(sar);
        sar[0]
    } else if let Some(mut sar) = state.sar {
        advance_sar(
            &mut sar,
            bar,
            start,
            inc,
            max,
            last_high.unwrap_or(f64::NAN),
            last_low.unwrap_or(f64::NAN),
            older_high,
            older_low,
            index,
        );
        state.sar = Some(sar);
        sar[0]
    } else {
        f64::NAN
    };
    state.highs.push(bar.high);
    state.lows.push(bar.low);
    state.closes.push(bar.close);
    if value.is_finite() {
        RuntimeValue::Float(value)
    } else {
        RuntimeValue::Na
    }
}

#[allow(clippy::too_many_arguments)]
fn advance_sar(
    state: &mut [f64; 4],
    bar: &Kline,
    start: f64,
    inc: f64,
    max: f64,
    last_high: f64,
    last_low: f64,
    older_high: Option<f64>,
    older_low: Option<f64>,
    index: usize,
) {
    let (mut sar, mut extreme, mut acceleration) = (state[0], state[1], state[2]);
    let mut below = state[3] == 1.0;
    let mut first_trend_bar = index == 1;
    sar += acceleration * (extreme - sar);
    if below && sar > bar.low {
        first_trend_bar = true;
        below = false;
        sar = bar.high.max(extreme);
        extreme = bar.low;
        acceleration = start;
    } else if !below && sar < bar.high {
        first_trend_bar = true;
        below = true;
        sar = bar.low.min(extreme);
        extreme = bar.high;
        acceleration = start;
    }
    if !first_trend_bar {
        if below && bar.high > extreme {
            extreme = bar.high;
            acceleration = (acceleration + inc).min(max);
        } else if !below && bar.low < extreme {
            extreme = bar.low;
            acceleration = (acceleration + inc).min(max);
        }
    }
    if below {
        sar = sar.min(last_low);
        if index > 1
            && let Some(older) = older_low
        {
            sar = sar.min(older);
        }
    } else {
        sar = sar.max(last_high);
        if index > 1
            && let Some(older) = older_high
        {
            sar = sar.max(older);
        }
    }
    *state = [sar, extreme, acceleration, if below { 1.0 } else { 0.0 }];
}

/// `ta.supertrend(factor, atrPeriod)`: `[supertrend, direction]`, direction 1 while the trend is
/// down and -1 while it is up. A port of the reference code TradingView prints for it, `nz()`
/// quirks included: the lower band holds 0 and the upper band is `na` until the ATR exists, so the
/// line is 0 on the first bar, `na` until the bar before the ATR's first, and real from there.
fn evaluate_supertrend(
    state: &mut State,
    bar: &Kline,
    previous_close: Option<f64>,
    factor: f64,
    period: f64,
) -> RuntimeValue {
    let atr_was_missing = state
        .results
        .last()
        .and_then(RuntimeValue::number)
        .is_none();
    let atr = evaluate_atr(state, bar, previous_close, period.max(1.0) as i64).number();

    let (previous_upper, previous_lower, previous_line) =
        state.supertrend.map_or((0.0, 0.0, f64::NAN), |previous| {
            (nz(previous.upper), nz(previous.lower), previous.line)
        });
    let source = (bar.high + bar.low) / 2.0;
    let raw_upper = atr.map_or(f64::NAN, |atr| source + factor * atr);
    let raw_lower = atr.map_or(f64::NAN, |atr| source - factor * atr);
    let closed_below = previous_close.is_some_and(|close| close < previous_lower);
    let closed_above = previous_close.is_some_and(|close| close > previous_upper);
    let lower = if raw_lower > previous_lower || closed_below {
        raw_lower
    } else {
        previous_lower
    };
    let upper = if raw_upper < previous_upper || closed_above {
        raw_upper
    } else {
        previous_upper
    };

    let direction: i64 = if atr_was_missing {
        1
    } else if previous_line == previous_upper {
        if bar.close > upper { -1 } else { 1 }
    } else if bar.close < lower {
        1
    } else {
        -1
    };
    let line = if direction == -1 { lower } else { upper };
    state.supertrend = Some(Supertrend { upper, lower, line });
    RuntimeValue::Tuple(vec![
        if line.is_nan() {
            RuntimeValue::Na
        } else {
            float(line)
        },
        RuntimeValue::Int(direction),
    ])
}

/// `nz(x)` for a `NaN`-as-`na` float.
fn nz(value: f64) -> f64 {
    if value.is_nan() { 0.0 } else { value }
}

/// `ta.tr(handle_na)`: the true range against the chart's previous close, `na` on a bar with none
/// (the first) unless `handle_na`, which gives the bar's high minus low.
pub fn true_range_checked(
    bar: &Kline,
    previous_close: Option<f64>,
    handle_na: bool,
) -> Option<f64> {
    match previous_close {
        Some(_) => Some(true_range(bar, previous_close)),
        None => handle_na.then_some(bar.high - bar.low),
    }
}

/// True range: the widest of the bar's range and its gaps from the previous close.
pub fn true_range(bar: &Kline, previous_close: Option<f64>) -> f64 {
    (bar.high - bar.low)
        .max(previous_close.map_or(0.0, |close| (bar.high - close).abs()))
        .max(previous_close.map_or(0.0, |close| (bar.low - close).abs()))
}

/// `fixnan(source)`: `source`, with `na` replaced by the last real value (`na` until there is one).
pub fn evaluate_fixnan(source: &RuntimeValue, state: &mut State) -> RuntimeValue {
    let result = if source.number().is_some() {
        source.clone()
    } else {
        state.results.last().cloned().unwrap_or(RuntimeValue::Na)
    };
    state.results.push(result.clone());
    result
}

/// `ta.cum(source)`: the running total of `source`; a `na` adds nothing and the total carries on,
/// like the Swift runtime.
pub fn evaluate_cum(source: &RuntimeValue, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let previous = state.results.last().and_then(RuntimeValue::number);
    // Na until the first value; after that a missing value adds nothing.
    let result = match (previous, source.number()) {
        (None, None) => RuntimeValue::Na,
        (previous, source) => float(previous.unwrap_or(0.0) + source.unwrap_or(0.0)),
    };
    state.results.push(result.clone());
    result
}

/// The open time of the bar before the one at `open_time` that the site ran on (`None` the first
/// time it runs), noting this bar. A site that runs several times in one bar (a loop) sees the same
/// earlier bar every time. `timeframe.change` compares with it.
pub fn previous_bar_time(open_time: i64, state: &mut State) -> Option<i64> {
    let Some(times) = state.bar_times.as_deref_mut() else {
        state.bar_times = Some(Box::new(BarTimes {
            latest: open_time,
            before: None,
        }));
        return None;
    };
    if times.latest != open_time {
        *times = BarTimes {
            latest: open_time,
            before: Some(times.latest),
        };
    }
    times.before
}

/// `ta.barssince(condition)`: bars since the condition was last true (0 on a true bar), `na`
/// before it ever was.
pub fn evaluate_barssince(condition: &RuntimeValue, state: &mut State) -> RuntimeValue {
    let result = if condition.bool() == Some(true) {
        RuntimeValue::Int(0)
    } else {
        match state.results.last().and_then(RuntimeValue::int_value) {
            Some(prior) => RuntimeValue::Int(prior + 1),
            None => RuntimeValue::Na,
        }
    };
    state.results.push(result.clone());
    result
}

/// `ta.vwap(source, anchor, stdev_mult)`: volume-weighted average price, restarting each UTC day
/// unless the script passes its own `anchor`. With a multiplier it is `[vwap, upper, lower]` using
/// the volume-weighted standard deviation. The state holds `[Σ price·volume, Σ volume,
/// Σ price²·volume, day]`.
pub fn evaluate_vwap(
    price: f64,
    bar: &Kline,
    anchor: Option<bool>,
    multiplier: Option<f64>,
    state: &mut State,
) -> RuntimeValue {
    let day = bar.open_time.div_euclid(86_400_000) as f64;
    let mut sums = match state.results.last() {
        Some(RuntimeValue::Tuple(parts)) if parts.len() == 4 => {
            [0, 1, 2, 3].map(|i| parts[i].number().unwrap_or(0.0))
        }
        _ => [0.0, 0.0, 0.0, day],
    };
    if anchor.unwrap_or(sums[3] != day) {
        sums = [0.0, 0.0, 0.0, day];
    }
    sums[0] += price * bar.volume;
    sums[1] += bar.volume;
    sums[2] += price * price * bar.volume;
    sums[3] = day;
    state
        .results
        .push(RuntimeValue::Tuple(sums.map(float).to_vec()));
    if sums[1] <= 0.0 {
        return if multiplier.is_some() {
            RuntimeValue::Tuple(vec![RuntimeValue::Na; 3])
        } else {
            RuntimeValue::Na
        };
    }
    let vwap = sums[0] / sums[1];
    let Some(multiplier) = multiplier else {
        return float(vwap);
    };
    let deviation = (sums[2] / sums[1] - vwap * vwap).max(0.0).sqrt();
    RuntimeValue::Tuple(vec![
        float(vwap),
        float(vwap + multiplier * deviation),
        float(vwap - multiplier * deviation),
    ])
}

/// The `ta.vwap` variable: `ta.vwap(hlc3)` with the default anchor. A script may read it on some
/// bars only, or several times on one (`ta.vwap[1]` reads it again at the bar's end), so the state
/// holds one entry per *committed* bar: a read first folds in every committed bar it has not seen,
/// from the session's histories, then computes the current bar and does not keep it.
pub fn evaluate_vwap_variable(
    bar: &Kline,
    runtime: &crate::pine::runtime::state::RuntimeState,
    state: &mut State,
) -> RuntimeValue {
    let series = |name: &str| runtime.histories.get(name).map_or(&[][..], Vec::as_slice);
    let (times, highs, lows) = (series("time"), series("high"), series("low"));
    let (closes, volumes) = (series("close"), series("volume"));
    let committed = closes.len();
    state.results.truncate(committed);
    let number = |values: &[RuntimeValue], index: usize| {
        values
            .get(index)
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN)
    };
    for index in state.results.len()..committed {
        let folded = Kline {
            open_time: times
                .get(index)
                .and_then(RuntimeValue::int_value)
                .unwrap_or(0),
            close_time: 0,
            open: 0.0,
            high: number(highs, index),
            low: number(lows, index),
            close: number(closes, index),
            volume: number(volumes, index),
        };
        let price = (folded.high + folded.low + folded.close) / 3.0;
        evaluate_vwap(price, &folded, None, None, state);
    }
    let price = (bar.high + bar.low + bar.close) / 3.0;
    let value = evaluate_vwap(price, bar, None, None, state);
    state.results.pop();
    value
}

/// `ta.dmi(diLength, adxSmoothing)`: `[+DI, -DI, ADX]` from the RMA-smoothed directional movement
/// and true range. The call site's own state remembers the previous bar's high and low and holds
/// the four smoothed series (the true range, both movements and the directional index).
/// `previous_close` is the chart's `close[1]`: the true range is `na` on the first bar, as
/// `ta.tr` is, so every average starts on the bar after it.
pub fn evaluate_dmi(
    state: &mut State,
    bar: &Kline,
    previous_close: Option<f64>,
    di_length: i64,
    adx_smoothing: i64,
) -> RuntimeValue {
    let previous_high = state.highs.last().copied();
    let previous_low = state.lows.last().copied();
    state.highs.push(bar.high);
    state.lows.push(bar.low);
    let (mut plus_move, mut minus_move) = (RuntimeValue::Na, RuntimeValue::Na);
    if let (Some(high), Some(low)) = (previous_high, previous_low) {
        let up = bar.high - high;
        let down = low - bar.low;
        plus_move = float(if up > down && up > 0.0 { up } else { 0.0 });
        minus_move = float(if down > up && down > 0.0 { down } else { 0.0 });
    }
    let smoothers = state.dmi.get_or_insert_with(Default::default);
    let mut smooth = |index: usize, value: RuntimeValue, length: i64| {
        evaluate(
            "ta.rma",
            &value,
            &RuntimeValue::Na,
            length,
            &mut smoothers[index],
        )
    };
    let range = smooth(
        0,
        true_range_checked(bar, previous_close, false).map_or(RuntimeValue::Na, float),
        di_length,
    );
    let plus_smoothed = smooth(1, plus_move, di_length);
    let minus_smoothed = smooth(2, minus_move, di_length);
    let (mut plus, mut minus, mut strength) =
        (RuntimeValue::Na, RuntimeValue::Na, RuntimeValue::Na);
    if let (Some(r), Some(p), Some(m)) = (
        range.number().filter(|r| *r != 0.0),
        plus_smoothed.number(),
        minus_smoothed.number(),
    ) {
        let (plus_di, minus_di) = (100.0 * p / r, 100.0 * m / r);
        let total = plus_di + minus_di;
        plus = float(plus_di);
        minus = float(minus_di);
        strength = float((plus_di - minus_di).abs() / if total == 0.0 { 1.0 } else { total });
    }
    let adx = smooth(3, strength, adx_smoothing)
        .number()
        .map_or(RuntimeValue::Na, |value| float(100.0 * value));
    RuntimeValue::Tuple(vec![plus, minus, adx])
}

/// `ta.correlation(source1, source2, length)`: Pearson's coefficient over the last `length` pairs;
/// `na` until the window is full of numbers, or when either series is flat.
pub fn evaluate_correlation(
    first: &RuntimeValue,
    second: &RuntimeValue,
    length: i64,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(first.clone());
    state.seconds.push(second.clone());
    let count = usize::try_from(length).unwrap_or(0);
    let result = (|| {
        let xs = last_window(&state.inputs, count)?;
        let ys = last_window(&state.seconds, count)?;
        let n = count as f64;
        let (mean_x, mean_y) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
        let (mut covariance, mut spread_x, mut spread_y) = (0.0, 0.0, 0.0);
        for (x, y) in xs.iter().zip(&ys) {
            covariance += (x - mean_x) * (y - mean_y);
            spread_x += (x - mean_x).powi(2);
            spread_y += (y - mean_y).powi(2);
        }
        let denominator = (spread_x * spread_y).sqrt();
        (denominator > 0.0).then(|| covariance / denominator)
    })();
    let result = optional(result);
    state.results.push(result.clone());
    result
}

/// `ta.atr(length)`: Wilder's RMA of the true range, seeded from the simple average of the first
/// `length` true ranges. The state's `inputs` hold the true-range series.
pub fn evaluate_atr(
    state: &mut State,
    bar: &Kline,
    previous_close: Option<f64>,
    length: i64,
) -> RuntimeValue {
    let range = true_range(bar, previous_close);
    state.inputs.push(float(range));
    let count = state.inputs.len();
    let length = usize::try_from(length).unwrap_or(0);
    let value = if length == 0 || count < length {
        None
    } else if count == length {
        Some(
            state
                .inputs
                .iter()
                .filter_map(RuntimeValue::number)
                .sum::<f64>()
                / length as f64,
        )
    } else {
        let previous = state.results.last().and_then(RuntimeValue::number);
        previous.map(|p| (p * (length as f64 - 1.0) + range) / length as f64)
    };
    let result = optional(value);
    state.results.push(result.clone());
    result
}

/// `ta.stdev(source, length, biased)` and `ta.variance(source, length, biased)`: the population
/// figure when `biased` (the default), the sample one (divided by `length - 1`) when not. A window
/// of one value has no sample figure.
pub fn evaluate_dispersion(
    name: &str,
    source: &RuntimeValue,
    length: i64,
    biased: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let count = usize::try_from(length).unwrap_or(0);
    let result = optional(last_valid(&state.inputs, count).and_then(|window| {
        let figure = if biased {
            variance(&window)
        } else if window.len() < 2 {
            return None;
        } else {
            variance(&window) * window.len() as f64 / (window.len() - 1) as f64
        };
        Some(if name == "ta.stdev" {
            figure.sqrt()
        } else {
            figure
        })
    }));
    state.results.push(result.clone());
    result
}

/// `ta.bb(source, length, mult)`: `[basis, upper, lower]`, the basis a simple average and the
/// bands `mult` population standard deviations away.
pub fn evaluate_bb(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let count = usize::try_from(length).unwrap_or(0);
    let basis = sma(&state.inputs, count).number();
    let deviation = stdev(&state.inputs, count).number();
    let bands = basis
        .zip(deviation)
        .map(|(b, d)| (b + multiplier * d, b - multiplier * d));
    let result = RuntimeValue::Tuple(vec![
        optional(basis),
        optional(bands.map(|(upper, _)| upper)),
        optional(bands.map(|(_, lower)| lower)),
    ]);
    state.results.push(result.clone());
    result
}

/// `ta.pivothigh` / `ta.pivotlow` (`source`, `left`, `right`): the value of the bar `right` bars
/// back when it is the extreme of the `left` bars before it and the `right` bars after it, else
/// `na`. A tie on the left side still qualifies; a tie on the right does not, like Pine.
pub fn evaluate_pivot(
    source: &RuntimeValue,
    left: i64,
    right: i64,
    high: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let (left, right) = (
        usize::try_from(left).unwrap_or(0),
        usize::try_from(right).unwrap_or(0),
    );
    let values = &state.inputs;
    let result = (|| {
        let newest = values.len().checked_sub(1)?;
        let center_index = newest.checked_sub(right)?;
        let start = center_index.checked_sub(left)?;
        let center = values[center_index].number()?;
        let beats = |other: f64, strict: bool| match (high, strict) {
            (true, true) => other >= center,
            (true, false) => other > center,
            (false, true) => other <= center,
            (false, false) => other < center,
        };
        for value in &values[start..center_index] {
            if beats(value.number()?, false) {
                return None;
            }
        }
        for value in &values[center_index + 1..=newest] {
            if beats(value.number()?, true) {
                return None;
            }
        }
        Some(center)
    })();
    let result = optional(result);
    state.results.push(result.clone());
    result
}

// MARK: Windows

/// The last `count` non-`na` inputs, oldest first, or None when fewer exist.
fn last_valid(inputs: &[RuntimeValue], count: usize) -> Option<Vec<f64>> {
    if count == 0 || count > inputs.len() {
        return None;
    }
    let mut found: Vec<f64> = inputs
        .iter()
        .rev()
        .filter_map(RuntimeValue::number)
        .take(count)
        .collect();
    if found.len() != count {
        return None;
    }
    found.reverse();
    Some(found)
}

/// The last `count` inputs as numbers, or None when there are fewer or any is `na`.
fn last_window(inputs: &[RuntimeValue], count: usize) -> Option<Vec<f64>> {
    if count == 0 || count > inputs.len() {
        return None;
    }
    let numbers: Vec<f64> = inputs[inputs.len() - count..]
        .iter()
        .filter_map(RuntimeValue::number)
        .collect();
    (numbers.len() == count).then_some(numbers)
}

fn latest(inputs: &[RuntimeValue]) -> Option<f64> {
    inputs.last().and_then(RuntimeValue::number)
}

// MARK: Indicators

fn sma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    optional(last_valid(inputs, length).map(|values| values.iter().sum::<f64>() / length as f64))
}

fn extremum(inputs: &[RuntimeValue], length: usize, pick: fn(f64, f64) -> f64) -> RuntimeValue {
    optional(last_valid(inputs, length).and_then(|values| values.into_iter().reduce(pick)))
}

fn change(inputs: &[RuntimeValue], offset: usize) -> RuntimeValue {
    if inputs.len() <= offset {
        return RuntimeValue::Na;
    }
    let (Some(current), Some(previous)) =
        (latest(inputs), inputs[inputs.len() - 1 - offset].number())
    else {
        return RuntimeValue::Na;
    };
    float(current - previous)
}

/// `ta.wma`: weights 1 to `length`, newest heaviest. A missing bar gives `na`; in the windows
/// after it the missing bar's slot holds the last value that existed before it (TradingView's
/// rule, measured), so the average is `na` only on the bar itself.
fn wma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 || length > inputs.len() || latest(inputs).is_none() {
        return RuntimeValue::Na;
    }
    let start = inputs.len() - length;
    let mut held: Option<f64> = None;
    let mut weighted = 0.0;
    for (offset, input) in inputs[start..].iter().enumerate() {
        let value = match input.number() {
            Some(value) => Some(value),
            None if offset == 0 => inputs[..start].iter().rev().find_map(RuntimeValue::number),
            None => held,
        };
        let Some(value) = value else {
            return RuntimeValue::Na;
        };
        held = Some(value);
        weighted += (offset + 1) as f64 * value;
    }
    float(weighted / (length * (length + 1) / 2) as f64)
}

fn stdev(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    let Some(window) = last_valid(inputs, length) else {
        return RuntimeValue::Na;
    };
    float(variance(&window).sqrt())
}

fn trend(inputs: &[RuntimeValue], length: usize, rising: bool) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Bool(false);
    }
    let Some(window) = last_window(inputs, length + 1) else {
        return RuntimeValue::Bool(false);
    };
    RuntimeValue::Bool(window.windows(2).all(|pair| {
        if rising {
            pair[1] > pair[0]
        } else {
            pair[1] < pair[0]
        }
    }))
}

fn momentum(inputs: &[RuntimeValue], length: usize, is_rate: bool) -> RuntimeValue {
    if length == 0 || inputs.len() <= length {
        return RuntimeValue::Na;
    }
    let (Some(current), Some(previous)) =
        (latest(inputs), inputs[inputs.len() - 1 - length].number())
    else {
        return RuntimeValue::Na;
    };
    if !is_rate {
        return float(current - previous);
    }
    if previous == 0.0 {
        RuntimeValue::Na
    } else {
        float(100.0 * (current - previous) / previous)
    }
}

fn cross(
    name: &str,
    source: &RuntimeValue,
    second: &RuntimeValue,
    prior: Option<&RuntimeValue>,
) -> RuntimeValue {
    let no = RuntimeValue::Bool(false);
    let Some(RuntimeValue::Tuple(previous)) = prior else {
        return no;
    };
    let (Some(a), Some(b)) = (source.number(), second.number()) else {
        return no;
    };
    let [first, other] = previous.as_slice() else {
        return no;
    };
    let (Some(pa), Some(pb)) = (first.number(), other.number()) else {
        return no;
    };
    let crossed_over = a > b && pa <= pb;
    let crossed_under = a < b && pa >= pb;
    RuntimeValue::Bool(match name {
        "ta.crossover" => crossed_over,
        "ta.crossunder" => crossed_under,
        _ => crossed_over || crossed_under,
    })
}

// MARK: MACD

/// `ta.macd(source, fastlen, slowlen, siglen)`: `[macd, signal, histogram]`. The line is the fast
/// EMA minus the slow one, the signal an EMA of the line. A `na` source bar gives `na` for all
/// three and leaves every average as it was; each average is seeded with the simple average of its
/// first `length` values, so the line starts on bar `slowlen - 1` and the signal `siglen - 1` line
/// values later.
pub fn evaluate_macd(
    source: &RuntimeValue,
    fast: i64,
    slow: i64,
    signal: i64,
    state: &mut State,
) -> RuntimeValue {
    let value = source.number();
    let length = |length: i64| usize::try_from(length).unwrap_or(0);
    let fast = derived(state, 0).ema(value, length(fast));
    let slow = derived(state, 1).ema(value, length(slow));
    let line = fast.zip(slow).map(|(fast, slow)| fast - slow);
    let signal = derived(state, 2).ema(line, length(signal));
    RuntimeValue::Tuple(vec![
        optional(line),
        optional(signal),
        optional(line.zip(signal).map(|(line, signal)| line - signal)),
    ])
}

/// `ta.linreg(source, length, offset)`: the least-squares line through the last `length` values,
/// read `offset` bars before the newest (0 is the current bar, a negative offset projects forward).
fn linreg(inputs: &[RuntimeValue], length: usize, offset: i64) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let count = length as f64;
    let mean_x = (count - 1.0) / 2.0;
    let mean_y = window.iter().sum::<f64>() / count;
    let (mut covariance, mut spread) = (0.0, 0.0);
    for (index, value) in window.iter().enumerate() {
        let dx = index as f64 - mean_x;
        covariance += dx * (value - mean_y);
        spread += dx * dx;
    }
    let slope = if spread == 0.0 {
        0.0
    } else {
        covariance / spread
    };
    let intercept = mean_y - slope * mean_x;
    float(intercept + slope * (count - 1.0 - offset as f64))
}

/// `ta.max` / `ta.min`: the highest or lowest value the series has had so far; `na` does not
/// change it.
fn running_extreme(
    source: &RuntimeValue,
    prior: Option<&RuntimeValue>,
    pick: fn(f64, f64) -> f64,
) -> RuntimeValue {
    let previous = prior.and_then(RuntimeValue::number);
    let Some(value) = source.number() else {
        return optional(previous);
    };
    float(previous.map_or(value, |previous| pick(previous, value)))
}

// MARK: Window statistics

/// `compute` over the last `length` inputs, all of which must be numbers.
fn statistic(
    inputs: &[RuntimeValue],
    length: usize,
    compute: impl FnOnce(&[f64]) -> f64,
) -> RuntimeValue {
    optional(last_window(inputs, length).map(|window| compute(&window)))
}

/// `compute` over the last `length` inputs that exist: a missing bar is skipped, and on it the
/// statistic keeps the value of the bar before (TradingView's rule for `ta.stdev`, `ta.median`,
/// `ta.range` and `math.sum`; `ta.dev` is strict there).
fn skipping_statistic(
    inputs: &[RuntimeValue],
    length: usize,
    compute: impl FnOnce(&[f64]) -> f64,
) -> RuntimeValue {
    optional(last_valid(inputs, length).map(|window| compute(&window)))
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// The population variance, as `ta.variance` computes by default.
fn variance(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64
}

fn mean_deviation(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).abs()).sum::<f64>() / values.len() as f64
}

/// Chande momentum oscillator: the share of the net movement in all movement over `length`
/// changes.
fn cmo(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Na;
    }
    let Some(window) = last_window(inputs, length + 1) else {
        return RuntimeValue::Na;
    };
    let changes: Vec<f64> = window.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let up: f64 = changes.iter().filter(|c| **c > 0.0).sum();
    let down: f64 = -changes.iter().filter(|c| **c < 0.0).sum::<f64>();
    // Nothing moved: 0 / 0, which is `na` on TradingView.
    optional((up + down != 0.0).then(|| 100.0 * (up - down) / (up + down)))
}

/// Commodity channel index: the distance from the mean in units of 0.015 mean deviations. A window
/// of equal values has no deviation to divide by, and TradingView repeats the previous value there.
fn cci(inputs: &[RuntimeValue], length: usize, previous: Option<&RuntimeValue>) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    if window.iter().all(|value| *value == window[0]) {
        return previous.cloned().unwrap_or(RuntimeValue::Na);
    }
    let current = window[window.len() - 1];
    let mean = window.iter().sum::<f64>() / length as f64;
    float((current - mean) / (0.015 * mean_deviation(&window)))
}

/// The weighted average of the `length` values ending at `end` (inclusive), or None.
fn weighted_average(values: &[Option<f64>], end: usize, length: usize) -> Option<f64> {
    if length == 0 || end + 1 < length || end >= values.len() {
        return None;
    }
    let mut weighted = 0.0;
    for (offset, index) in (end + 1 - length..=end).enumerate() {
        weighted += (offset + 1) as f64 * values[index]?;
    }
    Some(weighted / (length * (length + 1) / 2) as f64)
}

/// Hull moving average: a weighted average (length √n) of 2 · wma(n/2) − wma(n).
fn hma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length <= 1 {
        return RuntimeValue::Na;
    }
    let values: Vec<Option<f64>> = inputs.iter().map(RuntimeValue::number).collect();
    let half = (length / 2).max(1);
    let smoothing = ((length as f64).sqrt() as usize).max(1);
    let last = values.len() - 1;
    if last + 1 < smoothing {
        return RuntimeValue::Na;
    }
    let mut difference: Vec<Option<f64>> = vec![None; values.len()];
    for (index, slot) in difference.iter_mut().enumerate().skip(last + 1 - smoothing) {
        if let (Some(fast), Some(slow)) = (
            weighted_average(&values, index, half),
            weighted_average(&values, index, length),
        ) {
            *slot = Some(2.0 * fast - slow);
        }
    }
    optional(weighted_average(&difference, last, smoothing))
}

/// Bars back to the highest (or lowest) of the last `length` inputs: 0 for the current bar,
/// negative further back, the most recent one on a tie.
fn extremum_bars(inputs: &[RuntimeValue], length: usize, highest: bool) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let mut best = 0;
    for back in 0..length {
        let value = window[length - 1 - back];
        let current = window[length - 1 - best];
        if if highest {
            value > current
        } else {
            value < current
        } {
            best = back;
        }
    }
    RuntimeValue::Int(-(best as i64))
}

/// The share, in percent, of the previous `length` values that are at or below the current one.
fn percent_rank(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 || inputs.len() <= length {
        return RuntimeValue::Na;
    }
    let Some(current) = latest(inputs) else {
        return RuntimeValue::Na;
    };
    let previous: Vec<f64> = inputs[inputs.len() - 1 - length..inputs.len() - 1]
        .iter()
        .filter_map(RuntimeValue::number)
        .collect();
    if previous.len() != length {
        return RuntimeValue::Na;
    }
    float(previous.iter().filter(|value| **value <= current).count() as f64 / length as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `name` over `series` one bar at a time, as the session does.
    fn run(name: &str, series: &[f64], length: i64) -> Vec<RuntimeValue> {
        let mut state = State::default();
        series
            .iter()
            .map(|value| evaluate(name, &float(*value), &RuntimeValue::Na, length, &mut state))
            .collect()
    }

    fn numbers(values: &[RuntimeValue]) -> Vec<Option<f64>> {
        values.iter().map(RuntimeValue::number).collect()
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn sma_needs_a_full_window_and_slides() {
        let result = numbers(&run("ta.sma", &[1.0, 2.0, 3.0, 4.0, 5.0], 3));
        assert_eq!(result, [None, None, Some(2.0), Some(3.0), Some(4.0)]);
        assert_eq!(
            numbers(&run("ta.sma", &[1.0, 2.0], 0)),
            [None, None],
            "no window: na"
        );
        assert_eq!(numbers(&run("ta.sma", &[1.0, 2.0], -3)), [None, None]);
    }

    #[test]
    fn a_gap_in_the_source_delays_a_window_that_skips_na() {
        let mut state = State::default();
        let mut feed =
            |value: RuntimeValue| evaluate("ta.sma", &value, &RuntimeValue::Na, 2, &mut state);
        assert_eq!(feed(float(2.0)), RuntimeValue::Na);
        assert_eq!(feed(RuntimeValue::Na), RuntimeValue::Na);
        // sma looks at the last two non-na values (2 and 4), skipping the gap.
        assert_eq!(feed(float(4.0)), float(3.0));
        // wma is na on the missing bar, and after it the bar's slot holds the value before it.
        let mut wma = State::default();
        let results: Vec<RuntimeValue> = [float(1.0), RuntimeValue::Na, float(3.0)]
            .iter()
            .map(|value| evaluate("ta.wma", value, &RuntimeValue::Na, 2, &mut wma))
            .collect();
        assert_eq!(results[1], RuntimeValue::Na);
        assert_eq!(results[2], float((1.0 + 2.0 * 3.0) / 3.0));
    }

    #[test]
    fn ema_seeds_from_the_sma_then_recurses() {
        let result = numbers(&run("ta.ema", &[2.0, 4.0, 6.0, 8.0], 2));
        assert_eq!(result[0], None);
        assert_eq!(result[1], Some(3.0), "the simple average seeds it");
        let alpha = 2.0 / 3.0;
        assert!(close(result[2], alpha * 6.0 + (1.0 - alpha) * 3.0));
        let rma = numbers(&run("ta.rma", &[2.0, 4.0, 6.0], 2));
        assert!(
            close(rma[2], 0.5 * 6.0 + 0.5 * 3.0),
            "rma uses alpha = 1/length"
        );
        assert_eq!(numbers(&run("ta.ema", &[1.0], 0)), [None]);
    }

    #[test]
    fn extremes_over_a_window_and_running_extremes() {
        assert_eq!(
            numbers(&run("ta.highest", &[1.0, 5.0, 3.0, 2.0], 2)),
            [None, Some(5.0), Some(5.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.lowest", &[4.0, 2.0, 3.0, 5.0], 2)),
            [None, Some(2.0), Some(2.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.max", &[1.0, 3.0, 2.0], 0)),
            [Some(1.0), Some(3.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.min", &[3.0, 1.0, 2.0], 0)),
            [Some(3.0), Some(1.0), Some(1.0)]
        );
        let mut state = State::default();
        evaluate("ta.max", &float(5.0), &RuntimeValue::Na, 0, &mut state);
        let kept = evaluate(
            "ta.max",
            &RuntimeValue::Na,
            &RuntimeValue::Na,
            0,
            &mut state,
        );
        assert_eq!(kept, float(5.0), "na does not change a running extreme");
    }

    #[test]
    fn change_momentum_and_rate_of_change() {
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 1)),
            [None, Some(3.0), Some(5.0)]
        );
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 0)),
            [None, Some(3.0), Some(5.0)],
            "0 reads as 1"
        );
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 2)),
            [None, None, Some(8.0)]
        );
        assert_eq!(
            numbers(&run("ta.mom", &[10.0, 12.0, 15.0], 2)),
            [None, None, Some(5.0)]
        );
        let roc = numbers(&run("ta.roc", &[10.0, 12.0, 15.0], 2));
        assert!(close(roc[2], 50.0));
        assert_eq!(
            numbers(&run("ta.roc", &[0.0, 1.0], 1)),
            [None, None],
            "division by zero is na"
        );
    }

    #[test]
    fn rising_and_falling_need_every_step_to_agree() {
        let rising = run("ta.rising", &[1.0, 2.0, 3.0, 3.0, 4.0], 2);
        assert_eq!(
            rising,
            [false, false, true, false, false]
                .map(RuntimeValue::Bool)
                .to_vec()
        );
        let falling = run("ta.falling", &[5.0, 4.0, 3.0, 4.0], 2);
        assert_eq!(falling[2], RuntimeValue::Bool(true));
        assert_eq!(falling[3], RuntimeValue::Bool(false));
        assert_eq!(
            run("ta.rising", &[1.0, 2.0], 0)[1],
            RuntimeValue::Bool(false)
        );
    }

    #[test]
    fn weighted_and_dispersion_statistics_over_a_window() {
        let wma = numbers(&run("ta.wma", &[1.0, 2.0, 3.0], 3));
        assert!(close(wma[2], (1.0 + 4.0 + 9.0) / 6.0));
        let dispersion = |name: &str, biased: bool| {
            let mut state = State::default();
            let series = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
            let results: Vec<RuntimeValue> = series
                .iter()
                .map(|value| evaluate_dispersion(name, &float(*value), 8, biased, &mut state))
                .collect();
            numbers(&results)
        };
        assert!(
            close(dispersion("ta.stdev", true)[7], 2.0),
            "population standard deviation"
        );
        assert!(close(dispersion("ta.variance", true)[7], 4.0));
        // `biased = false` divides by n - 1.
        assert!(close(dispersion("ta.variance", false)[7], 32.0 / 7.0));
        assert!(close(
            dispersion("ta.stdev", false)[7],
            (32.0_f64 / 7.0).sqrt()
        ));
        assert!(close(
            numbers(&run("ta.dev", &[1.0, 2.0, 3.0, 4.0], 4))[3],
            1.0
        ));
        assert!(close(
            numbers(&run("ta.median", &[5.0, 1.0, 3.0], 3))[2],
            3.0
        ));
        assert!(close(
            numbers(&run("ta.median", &[4.0, 1.0, 3.0, 2.0], 4))[3],
            2.5
        ));
        assert!(close(
            numbers(&run("ta.range", &[2.0, 9.0, 5.0], 3))[2],
            7.0
        ));
        assert!(close(
            numbers(&run("math.sum", &[1.0, 2.0, 3.0, 4.0], 3))[3],
            9.0
        ));
        assert!(close(
            numbers(&run("ta.swma", &[1.0, 2.0, 3.0, 4.0], 0))[3],
            (1.0 + 4.0 + 6.0 + 4.0) / 6.0
        ));
    }

    #[test]
    fn rsi_smooths_gains_and_losses_with_the_rma() {
        let up = numbers(&run("ta.rsi", &[1.0, 2.0, 3.0, 4.0], 3));
        assert_eq!(up[3], Some(100.0), "no losses: 100");
        assert_eq!(up[2], None, "the first value needs `length` changes");
        // Changes +1, -0.5, +1: seeds gain 2/3 and loss 1/6, so rs = 4 and the RSI is 80.
        let mixed = numbers(&run("ta.rsi", &[10.0, 11.0, 10.5, 11.5, 11.0], 3));
        assert!(close(mixed[3], 80.0));
        // A fall of 0.5 moves the averages by 1/3: gain 4/9, loss 5/18, so rs = 1.6.
        assert!(close(mixed[4], 100.0 - 100.0 / 2.6));
        assert_eq!(numbers(&run("ta.rsi", &[1.0, 2.0], 0)), [None, None]);
    }

    #[test]
    fn rsi_follows_wilders_published_fourteen_day_series() {
        // The closes of Wilder's worked example (as StockCharts reproduces it). Seeded with the
        // simple average of the first 14 changes and smoothed with alpha 1/14, the RSI reads
        // 70.46 on the 15th close; a window average (the old behaviour) reads 100 or 0 here.
        let closes = [
            44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08, 45.89, 46.03,
            45.61, 46.28, 46.28, 46.00, 46.03, 46.41, 46.22, 45.64, 46.21, 46.25, 45.71, 46.45,
            45.78, 45.35, 44.03, 44.18, 44.22, 44.57, 43.42, 42.66, 43.13,
        ];
        let expected = [
            70.46, 66.25, 66.48, 69.35, 66.29, 57.92, 62.88, 63.21, 56.01, 62.34, 54.67, 50.39,
            40.02, 41.49, 41.90, 45.50, 37.32, 33.09, 37.79,
        ];
        let values = numbers(&run("ta.rsi", &closes, 14));
        assert!(values[..14].iter().all(Option::is_none), "needs 14 changes");
        for (offset, want) in expected.iter().enumerate() {
            let got = values[14 + offset].expect("a value");
            assert!(
                (got - want).abs() < 0.005,
                "bar {}: {got} vs {want}",
                14 + offset
            );
        }
    }

    #[test]
    fn rsi_after_a_long_fall_is_low_but_not_zero() {
        // The simple average over the window read exactly 0 after `length` falls in a row; the
        // smoothed gain only decays, so a later bounce still moves the value.
        let mut prices = vec![100.0, 101.0, 100.5, 101.5, 101.0];
        prices.extend((1..=12).map(|step| 101.0 - f64::from(step)));
        let values = numbers(&run("ta.rsi", &prices, 3));
        let last = values[prices.len() - 1].expect("a value");
        assert!(last > 0.0 && last < 5.0, "{last}");
    }

    /// `ta.rsi` over a series that may hold `na`, one bar at a time.
    fn run_rsi_with_gaps(series: &[Option<f64>], length: i64) -> Vec<Option<f64>> {
        let mut state = State::default();
        series
            .iter()
            .map(|value| {
                let source = value.map_or(RuntimeValue::Na, float);
                evaluate("ta.rsi", &source, &RuntimeValue::Na, length, &mut state).number()
            })
            .collect()
    }

    #[test]
    fn rsi_has_no_value_while_its_change_is_missing_and_keeps_its_averages() {
        let gaps = [
            Some(1.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            None,
            Some(6.0),
            Some(7.0),
            Some(6.0),
        ];
        // The missing bar and the one after it have no change, so no value; the averages are as
        // they were when the changes come back.
        let values = run_rsi_with_gaps(&gaps, 3);
        assert_eq!(&values[..5], [None, None, None, Some(100.0), None]);
        assert_eq!(values[5], None, "the change across the gap is missing too");
        assert_eq!(values[6], Some(100.0));
        let loss = values[7].expect("a loss now");
        assert!((loss - (100.0 - 100.0 / (1.0 + 2.0 / 3.0 / (1.0 / 3.0)))).abs() < 1e-9);
        let gaps = [Some(1.0), None, Some(2.0), None, Some(3.0)];
        assert_eq!(
            run_rsi_with_gaps(&gaps, 3),
            [None; 5],
            "no change ever exists"
        );
        // Leading `na` (a warm-up) changes nothing: same values as without it.
        let plain = numbers(&run("ta.rsi", &[10.0, 11.0, 10.5, 11.5, 11.0], 3));
        let mut padded = vec![None, None];
        padded.extend([10.0, 11.0, 10.5, 11.5, 11.0].map(Some));
        assert_eq!(run_rsi_with_gaps(&padded, 3)[2..], plain);
    }

    #[test]
    fn cross_compares_this_bar_to_the_last() {
        let mut state = State::default();
        let mut feed =
            |name: &str, a: f64, b: f64| evaluate(name, &float(a), &float(b), 0, &mut state);
        assert_eq!(
            feed("ta.crossover", 1.0, 2.0),
            RuntimeValue::Bool(false),
            "no previous bar"
        );
        assert_eq!(feed("ta.crossover", 3.0, 2.0), RuntimeValue::Bool(true));
        assert_eq!(
            feed("ta.crossover", 4.0, 2.0),
            RuntimeValue::Bool(false),
            "already above"
        );
        assert_eq!(feed("ta.crossunder", 1.0, 2.0), RuntimeValue::Bool(true));
        assert_eq!(
            feed("ta.cross", 3.0, 2.0),
            RuntimeValue::Bool(true),
            "either way"
        );
        assert_eq!(feed("ta.cross", 3.5, 2.0), RuntimeValue::Bool(false));
        // Touching counts as the earlier side: 2 <= 2 then 3 > 2 crosses over.
        assert_eq!(feed("ta.crossunder", 2.0, 2.0), RuntimeValue::Bool(false));
        assert_eq!(feed("ta.crossover", 3.0, 2.0), RuntimeValue::Bool(true));
        // A pair with an `na` never crosses.
        let mut state = State::default();
        evaluate("ta.cross", &float(1.0), &float(2.0), 0, &mut state);
        let na = evaluate("ta.cross", &RuntimeValue::Na, &float(2.0), 0, &mut state);
        assert_eq!(na, RuntimeValue::Bool(false));
    }

    #[test]
    fn macd_needs_slow_history_then_a_signal() {
        let series: Vec<f64> = (1..=40).map(f64::from).collect();
        let mut state = State::default();
        let mut results = Vec::new();
        for value in &series {
            let [fast, slow, signal] = MACD_DEFAULTS;
            results.push(evaluate_macd(
                &float(*value),
                fast,
                slow,
                signal,
                &mut state,
            ));
        }
        let parts = |result: &RuntimeValue| match result {
            RuntimeValue::Tuple(parts) => numbers(parts),
            other => panic!("expected a tuple, got {other:?}"),
        };
        assert_eq!(
            parts(&results[24]),
            [None, None, None],
            "slow EMA not ready"
        );
        let first = parts(&results[25]);
        assert!(
            first[0].is_some() && first[1].is_none(),
            "line but no signal yet"
        );
        let later = parts(&results[39]);
        assert!(later.iter().all(Option::is_some), "{later:?}");
        assert!(
            close(later[2], later[0].unwrap() - later[1].unwrap()),
            "histogram = line − signal"
        );
        assert!(
            later[0].unwrap() > 0.0,
            "a rising series has a positive MACD line"
        );
    }

    #[test]
    fn macd_uses_the_lengths_it_is_given_and_skips_a_missing_bar() {
        let series: Vec<f64> = (1..=60).map(|i| f64::from(i * i % 17) + 100.0).collect();
        let feed = |values: &[Option<f64>]| {
            let mut state = State::default();
            values
                .iter()
                .map(|value| {
                    let source = value.map_or(RuntimeValue::Na, float);
                    match evaluate_macd(&source, 3, 6, 4, &mut state) {
                        RuntimeValue::Tuple(parts) => numbers(&parts),
                        other => panic!("expected a tuple, got {other:?}"),
                    }
                })
                .collect::<Vec<_>>()
        };
        let plain: Vec<Option<f64>> = series.iter().copied().map(Some).collect();
        let whole = feed(&plain);
        assert_eq!(whole[4], [None, None, None], "slow EMA not ready");
        assert!(whole[5][0].is_some() && whole[5][1].is_none());
        assert!(whole[8].iter().all(Option::is_some), "{:?}", whole[8]);

        // A bar without a source value is na for all three outputs, and the averages carry on as
        // if that bar had not been there.
        let mut gapped = plain.clone();
        gapped.insert(20, None);
        let with_gap = feed(&gapped);
        assert_eq!(with_gap[20], [None, None, None]);
        assert_eq!(with_gap[19], whole[19]);
        assert_eq!(with_gap[21], whole[20]);
        assert_eq!(with_gap[40], whole[39]);
    }

    #[test]
    fn oscillators_cmo_and_cci() {
        assert_eq!(numbers(&run("ta.cmo", &[1.0, 2.0, 3.0], 2))[2], Some(100.0));
        assert_eq!(
            numbers(&run("ta.cmo", &[1.0, 1.0, 1.0], 2))[2],
            None,
            "no movement is 0 / 0"
        );
        assert!(close(
            numbers(&run("ta.cmo", &[1.0, 3.0, 2.0], 2))[2],
            100.0 * (2.0 - 1.0) / 3.0
        ));
        let cci = numbers(&run("ta.cci", &[1.0, 2.0, 3.0], 3));
        assert!(close(cci[2], (3.0 - 2.0) / (0.015 * (2.0 / 3.0))));
        assert_eq!(
            numbers(&run("ta.cci", &[5.0, 5.0, 5.0], 3))[2],
            None,
            "flat from the start: nothing to repeat"
        );
        let held = numbers(&run("ta.cci", &[1.0, 2.0, 3.0, 3.0, 3.0, 3.0], 3));
        assert!(held[3].is_some());
        assert_eq!(held[4], held[3], "a flat window repeats the previous value");
        assert_eq!(held[5], held[3]);
    }

    #[test]
    fn extreme_bars_count_back_and_prefer_the_newest_on_a_tie() {
        let highest = run("ta.highestbars", &[1.0, 5.0, 3.0, 2.0], 3);
        assert_eq!(highest[3], RuntimeValue::Int(-2), "5 is two bars back");
        let tie = run("ta.highestbars", &[5.0, 1.0, 5.0], 3);
        assert_eq!(tie[2], RuntimeValue::Int(0), "the newest of equal highs");
        let lowest = run("ta.lowestbars", &[4.0, 1.0, 2.0], 3);
        assert_eq!(lowest[2], RuntimeValue::Int(-1));
        assert_eq!(highest[1], RuntimeValue::Na);
    }

    #[test]
    fn percent_rank_counts_prior_values_at_or_below() {
        let result = numbers(&run("ta.percentrank", &[1.0, 2.0, 3.0, 2.5], 3));
        assert_eq!(result[2], None, "needs length prior values");
        // Prior three: 1, 2, 3 vs current 2.5 → two are ≤ 2.5.
        assert!(close(result[3], 2.0 / 3.0 * 100.0));
        assert_eq!(numbers(&run("ta.percentrank", &[1.0], 0)), [None]);
    }

    #[test]
    fn linreg_fits_a_line_and_projects_with_the_offset() {
        let line = [2.0, 4.0, 6.0, 8.0];
        let mut state = State::default();
        let mut out = Vec::new();
        for value in line {
            out.push(evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(0),
                4,
                &mut state,
            ));
        }
        assert!(
            close(out[3].number(), 8.0),
            "a perfect line reads back its last value"
        );
        let mut state = State::default();
        let mut back = RuntimeValue::Na;
        for value in line {
            back = evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(1),
                4,
                &mut state,
            );
        }
        assert!(close(back.number(), 6.0), "offset 1 reads one bar back");
        let mut state = State::default();
        let mut ahead = RuntimeValue::Na;
        for value in line {
            ahead = evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(-1),
                4,
                &mut state,
            );
        }
        assert!(
            close(ahead.number(), 10.0),
            "a negative offset projects forward"
        );
    }

    #[test]
    fn hma_is_a_weighted_average_of_the_difference_of_two_wmas() {
        let series: Vec<f64> = (1..=20).map(f64::from).collect();
        let result = numbers(&run("ta.hma", &series, 9));
        assert!(
            result[..8].iter().all(Option::is_none),
            "needs length + sqrt(length) − 1 bars"
        );
        // On a straight line the Hull average has no lag at all: it reads the line itself.
        assert!(close(result[19], 20.0), "{:?}", result[19]);
        assert_eq!(numbers(&run("ta.hma", &series, 1)), vec![None; 20]);
    }

    #[test]
    fn unknown_names_are_na_and_the_supported_list_is_honest() {
        let mut state = State::default();
        assert_eq!(
            evaluate("ta.nope", &float(1.0), &RuntimeValue::Na, 3, &mut state),
            RuntimeValue::Na
        );
        assert_eq!(state.results.len(), 1, "every evaluation is recorded");
        assert_eq!(state.inputs.len(), 1);
        assert!(is_supported("ta.sma") && is_supported("ta.linreg") && is_supported("math.sum"));
        assert!(!is_supported("ta.sar"));
        // Every advertised name evaluates to something other than the unknown-name fallback path
        // for a long enough series.
        let series: Vec<f64> = (1..=60)
            .map(|n| f64::from(n % 7) + f64::from(n) * 0.1)
            .collect();
        for name in SUPPORTED {
            let mut state = State::default();
            let mut last = RuntimeValue::Na;
            for value in &series {
                last = evaluate(name, &float(*value), &float(2.0), 5, &mut state);
            }
            assert!(
                last != RuntimeValue::Na
                    || matches!(name, "ta.cross" | "ta.crossover" | "ta.crossunder"),
                "{name} never produced a value"
            );
        }
    }

    #[test]
    fn true_range_is_the_widest_of_the_range_and_the_gaps() {
        let bar = |high: f64, low: f64| Kline {
            open_time: 0,
            close_time: 0,
            open: low,
            high,
            low,
            close: high,
            volume: 0.0,
        };
        assert_eq!(
            true_range(&bar(10.0, 8.0), None),
            2.0,
            "the first bar has only its range"
        );
        assert_eq!(
            true_range(&bar(10.0, 8.0), Some(7.0)),
            3.0,
            "a gap up from the last close"
        );
        assert_eq!(true_range(&bar(10.0, 8.0), Some(12.0)), 4.0, "a gap down");
        assert_eq!(
            true_range(&bar(10.0, 8.0), Some(9.0)),
            2.0,
            "inside the range"
        );
    }

    fn kline(open_time: i64, low: f64, high: f64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 60_000,
            open: low,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn sar_tracks_below_price_in_an_uptrend() {
        // A steady uptrend: low/high/close all rise.
        let bars = [
            kline(0, 10.0, 12.0, 12.0),
            kline(1, 11.0, 13.0, 13.0),
            kline(2, 12.0, 14.0, 14.0),
            kline(3, 13.0, 15.0, 15.0),
        ];
        let mut state = State::default();
        let mut values = Vec::new();
        for bar in &bars {
            values.push(evaluate_price(
                "ta.sar",
                &[float(0.02), float(0.02), float(0.2)],
                bar,
                None,
                &mut state,
            ));
        }
        assert_eq!(values[0], RuntimeValue::Na, "the first bar has no SAR");
        for (value, bar) in values.iter().skip(1).zip(&bars[1..]) {
            let RuntimeValue::Float(sar) = value else {
                panic!("{value:?}");
            };
            assert!(sar.is_finite());
            assert!(*sar < bar.close, "SAR stays below the close in an uptrend");
        }
    }

    #[test]
    fn supertrend_is_a_line_and_a_direction_that_start_as_the_reference_does() {
        let bars: Vec<Kline> = (0..14_i64)
            .map(|index| {
                let climb = index as f64;
                kline(index, 10.0 + climb, 12.0 + climb, 12.0 + climb)
            })
            .collect();
        let mut state = State::default();
        let results: Vec<(Option<f64>, i64)> = bars
            .iter()
            .enumerate()
            .map(|(index, bar)| {
                let value = evaluate_price(
                    "ta.supertrend",
                    &[float(3.0), float(5.0)],
                    bar,
                    index.checked_sub(1).map(|before| bars[before].close),
                    &mut state,
                );
                let RuntimeValue::Tuple(parts) = value else {
                    panic!("expected [line, direction], got {value:?}");
                };
                let [line, RuntimeValue::Int(direction)] = parts.as_slice() else {
                    panic!("{parts:?}");
                };
                (line.number(), *direction)
            })
            .collect();
        // The upper band is the line while the ATR is missing: 0 on the first bar (the band holds
        // `nz(na)`), na until the bar before the ATR's first value, real from there.
        assert_eq!(results[0], (Some(0.0), 1));
        for (bar, result) in results.iter().enumerate().take(4).skip(1) {
            assert_eq!(*result, (None, 1), "bar {bar}");
        }
        assert!(results[4].0.is_some_and(f64::is_finite));
        // A steady climb flips to the up direction (-1) once the ATR exists.
        assert_eq!(results[13].1, -1);
    }

    fn run_atr(bars: &[Kline], length: i64) -> Vec<Option<f64>> {
        let mut state = State::default();
        bars.iter()
            .enumerate()
            .map(|(index, bar)| {
                let previous_close = index.checked_sub(1).map(|before| bars[before].close);
                evaluate_atr(&mut state, bar, previous_close, length).number()
            })
            .collect()
    }

    fn candle(high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open: close,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn atr_seeds_with_a_simple_average_then_smooths_like_wilder() {
        let bars = [
            candle(12.0, 8.0, 10.0),  // tr 4
            candle(13.0, 9.0, 12.0),  // tr 4
            candle(16.0, 11.0, 15.0), // tr 5 (gap up from 12 -> max(5, 4, 1))
            candle(15.0, 12.0, 13.0), // tr 3
        ];
        let atr = run_atr(&bars, 3);
        assert_eq!(atr[0], None);
        assert_eq!(atr[1], None);
        assert!((atr[2].unwrap() - 13.0 / 3.0).abs() < 1e-9);
        let expected = (13.0 / 3.0 * 2.0 + 3.0) / 3.0;
        assert!((atr[3].unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn bollinger_bands_are_basis_plus_minus_population_deviation() {
        let mut state = State::default();
        let mut last = RuntimeValue::Na;
        for value in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            last = evaluate_bb(&float(value), 8, 2.0, &mut state);
        }
        let RuntimeValue::Tuple(parts) = last else {
            panic!("a tuple");
        };
        let [basis, upper, lower] = [0, 1, 2].map(|i| parts[i].number().unwrap());
        assert!((basis - 5.0).abs() < 1e-9);
        assert!((upper - 9.0).abs() < 1e-9, "5 + 2 * 2");
        assert!((lower - 1.0).abs() < 1e-9);
        let early = evaluate_bb(&float(1.0), 20, 2.0, &mut State::default());
        assert!(matches!(early, RuntimeValue::Tuple(ref p) if p[0] == RuntimeValue::Na));
    }

    #[test]
    fn pivots_confirm_after_the_right_bars_and_ties_follow_pine() {
        let series = [1.0, 3.0, 2.0, 1.0, 5.0, 2.0, 1.0];
        let mut state = State::default();
        let found: Vec<Option<f64>> = series
            .iter()
            .map(|v| evaluate_pivot(&float(*v), 1, 1, true, &mut state).number())
            .collect();
        // 3.0 is confirmed one bar later (index 2); 5.0 at index 5.
        assert_eq!(found, [None, None, Some(3.0), None, None, Some(5.0), None]);

        let mut low_state = State::default();
        let lows: Vec<Option<f64>> = [3.0, 1.0, 2.0, 4.0]
            .iter()
            .map(|v| evaluate_pivot(&float(*v), 1, 1, false, &mut low_state).number())
            .collect();
        assert_eq!(lows, [None, None, Some(1.0), None]);

        // Equal highs: a left tie qualifies, a right tie does not.
        let mut tie_left = State::default();
        let left: Vec<Option<f64>> = [4.0, 4.0, 1.0]
            .iter()
            .map(|v| evaluate_pivot(&float(*v), 1, 1, true, &mut tie_left).number())
            .collect();
        assert_eq!(left[2], Some(4.0));
        let mut tie_right = State::default();
        let right: Vec<Option<f64>> = [1.0, 4.0, 4.0]
            .iter()
            .map(|v| evaluate_pivot(&float(*v), 1, 1, true, &mut tie_right).number())
            .collect();
        assert_eq!(right[2], None);
    }

    #[test]
    fn cum_totals_and_ignores_na() {
        let mut state = State::default();
        let run = |value: RuntimeValue, state: &mut State| evaluate_cum(&value, state);
        assert_eq!(run(float(2.0), &mut state), float(2.0));
        assert_eq!(run(float(3.0), &mut state), float(5.0));
        assert_eq!(
            run(RuntimeValue::Na, &mut state),
            float(5.0),
            "na adds nothing"
        );
        assert_eq!(run(float(4.0), &mut state), float(9.0));
    }

    #[test]
    fn barssince_counts_from_the_last_true() {
        let mut state = State::default();
        let mut run = |on: bool| evaluate_barssince(&RuntimeValue::Bool(on), &mut state);
        assert_eq!(run(false), RuntimeValue::Na);
        assert_eq!(run(true), RuntimeValue::Int(0));
        assert_eq!(run(false), RuntimeValue::Int(1));
        assert_eq!(run(false), RuntimeValue::Int(2));
        assert_eq!(run(true), RuntimeValue::Int(0));
    }

    #[test]
    fn vwap_restarts_each_utc_day_and_bands_use_the_weighted_deviation() {
        let bar = |time: i64, close: f64, volume: f64| Kline {
            open_time: time,
            close_time: time,
            open: close,
            high: close,
            low: close,
            close,
            volume,
        };
        let day = 86_400_000;
        let mut state = State::default();
        assert_eq!(
            evaluate_vwap(10.0, &bar(0, 10.0, 1.0), None, None, &mut state),
            float(10.0)
        );
        let second = evaluate_vwap(20.0, &bar(1_000, 20.0, 3.0), None, None, &mut state);
        assert_eq!(second, float(17.5), "(10·1 + 20·3) / 4");
        let next_day = evaluate_vwap(30.0, &bar(day, 30.0, 2.0), None, None, &mut state);
        assert_eq!(next_day, float(30.0), "a new UTC day starts over");
        let anchored = evaluate_vwap(40.0, &bar(day + 1, 40.0, 2.0), Some(true), None, &mut state);
        assert_eq!(anchored, float(40.0), "the script's own anchor restarts it");

        let mut banded = State::default();
        let RuntimeValue::Tuple(parts) =
            evaluate_vwap(10.0, &bar(0, 10.0, 1.0), None, Some(2.0), &mut banded)
        else {
            panic!("a tuple");
        };
        assert_eq!(
            parts,
            [float(10.0), float(10.0), float(10.0)],
            "no spread yet"
        );
        let none = evaluate_vwap(10.0, &bar(0, 10.0, 0.0), None, None, &mut State::default());
        assert_eq!(none, RuntimeValue::Na, "no volume, no vwap");
    }

    #[test]
    fn dmi_reads_an_uptrend_as_plus_over_minus() {
        let mut state = State::default();
        let mut last = RuntimeValue::Na;
        for i in 0..60 {
            let base = 100.0 + i as f64 * 2.0;
            let before = (i > 0).then_some(base - 2.0);
            last = evaluate_dmi(
                &mut state,
                &candle(base + 1.0, base - 1.0, base),
                before,
                14,
                14,
            );
        }
        let RuntimeValue::Tuple(parts) = last else {
            panic!("a tuple");
        };
        let [plus, minus, adx] = [0, 1, 2].map(|i| parts[i].number().expect("warmed up"));
        assert!(plus > minus, "{plus} vs {minus}");
        assert!(
            minus < 1.0e-9 && (0.0..=100.0).contains(&adx),
            "{minus} {adx}"
        );
        let early = evaluate_dmi(&mut State::default(), &candle(2.0, 1.0, 1.5), None, 14, 14);
        assert!(matches!(early, RuntimeValue::Tuple(ref p) if p[0] == RuntimeValue::Na));
    }

    #[test]
    fn correlation_is_pearson_over_the_window() {
        let mut state = State::default();
        let xs = [1.0, 2.0, 3.0, 4.0];
        let mut last = RuntimeValue::Na;
        for x in xs {
            last = evaluate_correlation(&float(x), &float(10.0 - 2.0 * x), 4, &mut state);
        }
        assert!(
            (last.number().unwrap() + 1.0).abs() < 1e-9,
            "a falling line is -1"
        );

        let mut same = State::default();
        for x in xs {
            last = evaluate_correlation(&float(x), &float(3.0 * x + 1.0), 4, &mut same);
        }
        assert!((last.number().unwrap() - 1.0).abs() < 1e-9);

        let mut flat = State::default();
        for x in xs {
            last = evaluate_correlation(&float(x), &float(5.0), 4, &mut flat);
        }
        assert_eq!(last, RuntimeValue::Na, "a flat series has no correlation");
        let early = evaluate_correlation(&float(1.0), &float(2.0), 4, &mut State::default());
        assert_eq!(early, RuntimeValue::Na);
    }

    #[test]
    fn fixnan_carries_the_last_real_value_forward() {
        let mut state = State::default();
        let mut run = |value: RuntimeValue| evaluate_fixnan(&value, &mut state);
        assert_eq!(run(RuntimeValue::Na), RuntimeValue::Na);
        assert_eq!(run(float(3.0)), float(3.0));
        assert_eq!(run(RuntimeValue::Na), float(3.0));
        assert_eq!(run(RuntimeValue::Na), float(3.0));
        assert_eq!(run(float(4.0)), float(4.0));
    }
}
