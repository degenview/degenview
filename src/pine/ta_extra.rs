//! The rest of the `ta.*` namespace: windowed statistics that need no bar data (`alma`, `cog`,
//! `mode`, `percentile_*`, `valuewhen`), the ones that read the bar's high/low/close/volume
//! (`mfi`, `vwma`, `stoch`, `wpr`, `kc`, `kcw`, `bbw`, `tsi`) and the cumulative volume variables
//! (`ta.obv`, `ta.accdist`, …). Formulas follow the Pine v6 reference.
//!
//! Every function is called once per bar per call site and remembers what it needs in the call
//! site's [`State`], so a realtime tick that is rolled back rolls these back with it.

use super::runtime_value::RuntimeValue;
use super::state::RuntimeState;
use super::ta::{self, State};
use crate::model::Kline;

/// The `ta.*` functions this module computes (parameters in `ta_call::parameters`).
pub const FUNCTIONS: [&str; 16] = [
    "ta.alma",
    "ta.cog",
    "ta.mode",
    "ta.percentile_nearest_rank",
    "ta.percentile_linear_interpolation",
    "ta.valuewhen",
    "ta.mfi",
    "ta.vwma",
    "ta.stoch",
    "ta.wpr",
    "ta.kc",
    "ta.kcw",
    "ta.bbw",
    "ta.tsi",
    "ta.rci",
    "ta.pivot_point_levels",
];

/// The `ta.*` variables [`cumulative`] computes. A variable's position is where its state lives.
pub const VARIABLES: [&str; 8] = [
    "ta.obv",
    "ta.accdist",
    "ta.iii",
    "ta.nvi",
    "ta.pvi",
    "ta.pvt",
    "ta.wad",
    "ta.wvad",
];

/// A series a function derives from its inputs (`ta.kc`'s two averages, `ta.tsi`'s four) with the
/// last result of its own moving average.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    values: Vec<Option<f64>>,
    last: Option<f64>,
}

impl Derived {
    /// The EMA of everything pushed so far, seeded with the simple average of the first `length`
    /// values like `ta.ema`.
    fn ema(&mut self, value: Option<f64>, length: usize) -> Option<f64> {
        self.values.push(value);
        let result = match (self.last, value) {
            _ if length == 0 => None,
            (Some(prior), Some(x)) => {
                let alpha = 2.0 / (length as f64 + 1.0);
                Some(alpha * x + (1.0 - alpha) * prior)
            }
            (Some(_), None) => None,
            (None, _) => self
                .window(length)
                .map(|w| w.iter().sum::<f64>() / length as f64),
        };
        if result.is_some() {
            self.last = result;
        } else if value.is_none() {
            // A gap breaks the average; it reseeds once `length` values are present again.
            self.last = None;
        }
        result
    }

    fn window(&self, length: usize) -> Option<Vec<f64>> {
        let start = self.values.len().checked_sub(length)?;
        self.values[start..].iter().copied().collect()
    }
}

/// The pivot point levels of `ta.pivot_point_levels`, in the order the function returns them.
pub const PIVOT_LEVELS: [&str; 11] = [
    "P", "R1", "S1", "R2", "S2", "R3", "S3", "R4", "S4", "R5", "S5",
];

/// The kinds of pivot point `ta.pivot_point_levels` takes as its `type`.
pub const PIVOT_TYPES: [&str; 6] = [
    "Traditional",
    "Fibonacci",
    "Woodie",
    "Classic",
    "DM",
    "Camarilla",
];

/// The open, high, low and close of the bars since an anchor, and the levels last worked out from
/// a finished period.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotPeriod {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    levels: [Option<f64>; 11],
}

impl PivotPeriod {
    fn starting_with(bar: &Kline) -> Self {
        Self {
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            levels: [None; 11],
        }
    }

    fn take(&mut self, bar: &Kline) {
        self.high = self.high.max(bar.high);
        self.low = self.low.min(bar.low);
        self.close = bar.close;
    }
}

/// `ta.pivot_point_levels(type, anchor, developing)`: `[P, R1, S1, R2, S2, R3, S3, R4, S4, R5, S5]`
/// for the period that ran from the previous anchor to this one, with `na` for a level the `type`
/// has none of. A period starts at each bar where `anchor` is true (and at the first bar); the
/// levels set when one ends hold until the next ends, and are `na` before the first. With
/// `developing` they are instead worked out from the bars of the running period so far, this bar
/// included. Woodie's pivot weighs the open of the period that follows (the bar that sets the levels)
/// in the same way, so a developing Woodie uses the running period's own open.
///
/// `None` when `kind` is not one of [`PIVOT_TYPES`].
pub fn pivot_point_levels(
    kind: &str,
    anchor: bool,
    developing: bool,
    bar: &Kline,
    state: &mut State,
) -> Option<[Option<f64>; 11]> {
    if !PIVOT_TYPES.contains(&kind) {
        return None;
    }
    match state.pivot.as_mut() {
        Some(period) if anchor => {
            let levels = pivot_levels(kind, period, bar.open);
            let mut next = PivotPeriod::starting_with(bar);
            next.levels = levels;
            *period = next;
        }
        Some(period) => period.take(bar),
        None => state.pivot = Some(PivotPeriod::starting_with(bar)),
    }
    let period = state.pivot.as_ref()?;
    Some(if developing {
        pivot_levels(kind, period, period.open)
    } else {
        period.levels
    })
}

/// The levels of `kind` for a period; `next_open` is the open Woodie's pivot uses.
fn pivot_levels(kind: &str, period: &PivotPeriod, next_open: f64) -> [Option<f64>; 11] {
    let (open, high, low, close) = (period.open, period.high, period.low, period.close);
    let range = high - low;
    let mut levels = [None; 11];
    let mut set = |name: &str, value: f64| {
        if let Some(slot) = PIVOT_LEVELS.iter().position(|level| *level == name) {
            levels[slot] = Some(value);
        }
    };
    let typical = (high + low + close) / 3.0;
    match kind {
        "Traditional" => {
            set("P", typical);
            set("R1", typical * 2.0 - low);
            set("S1", typical * 2.0 - high);
            set("R2", typical + range);
            set("S2", typical - range);
            set("R3", typical * 2.0 + (high - 2.0 * low));
            set("S3", typical * 2.0 - (2.0 * high - low));
            set("R4", typical * 3.0 + (high - 3.0 * low));
            set("S4", typical * 3.0 - (3.0 * high - low));
            set("R5", typical * 4.0 + (high - 4.0 * low));
            set("S5", typical * 4.0 - (4.0 * high - low));
        }
        "Fibonacci" => {
            set("P", typical);
            set("R1", typical + 0.382 * range);
            set("S1", typical - 0.382 * range);
            set("R2", typical + 0.618 * range);
            set("S2", typical - 0.618 * range);
            set("R3", typical + range);
            set("S3", typical - range);
        }
        "Woodie" => {
            let pivot = (high + low + 2.0 * next_open) / 4.0;
            set("P", pivot);
            set("R1", pivot * 2.0 - low);
            set("S1", pivot * 2.0 - high);
            set("R2", pivot + range);
            set("S2", pivot - range);
            let (third_up, third_down) = (high + 2.0 * (pivot - low), low - 2.0 * (high - pivot));
            set("R3", third_up);
            set("S3", third_down);
            set("R4", third_up + range);
            set("S4", third_down - range);
        }
        "Classic" => {
            set("P", typical);
            set("R1", typical * 2.0 - low);
            set("S1", typical * 2.0 - high);
            for (multiple, up, down) in [(1.0, "R2", "S2"), (2.0, "R3", "S3"), (3.0, "R4", "S4")] {
                set(up, typical + multiple * range);
                set(down, typical - multiple * range);
            }
        }
        "DM" => {
            let x = if close < open {
                high + 2.0 * low + close
            } else if close > open {
                2.0 * high + low + close
            } else {
                high + low + 2.0 * close
            };
            set("P", x / 4.0);
            set("R1", x / 2.0 - low);
            set("S1", x / 2.0 - high);
        }
        _ => {
            // Camarilla.
            set("P", typical);
            for (fraction, up, down) in [
                (1.1 / 12.0, "R1", "S1"),
                (1.1 / 6.0, "R2", "S2"),
                (1.1 / 4.0, "R3", "S3"),
                (1.1 / 2.0, "R4", "S4"),
            ] {
                set(up, close + range * fraction);
                set(down, close - range * fraction);
            }
            let upper = high / low * close;
            set("R5", upper);
            set("S5", close - (upper - close));
        }
    }
    levels
}

/// `ta.rci(source, length)`: Spearman's rank correlation between the last `length` values of the
/// source and their bar order, from -100 (always falling) to 100 (always rising). Equal values
/// share the mean of the ranks they span. `na` while the window is short or holds a `na`, and when
/// every value is equal (the correlation is undefined).
pub fn rci(source: &RuntimeValue, length: i64, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let length = count(length);
    let Some(values) = window(&state.inputs, length) else {
        return RuntimeValue::Na;
    };
    if length < 2 {
        return RuntimeValue::Na;
    }
    let ranks = mean_ranks(&values);
    // The bar order is 1…n and has the same mean and spread as any ranking without ties.
    let n = length as f64;
    let mean = (n + 1.0) / 2.0;
    let (mut covariance, mut spread_time, mut spread_value) = (0.0, 0.0, 0.0);
    for (index, rank) in ranks.iter().enumerate() {
        let time = index as f64 + 1.0 - mean;
        covariance += time * (rank - mean);
        spread_time += time * time;
        spread_value += (rank - mean) * (rank - mean);
    }
    out(Some(
        covariance / (spread_time * spread_value).sqrt() * 100.0,
    ))
}

/// The rank of each value among them, counting from 1 for the smallest; equal values get the mean
/// of the ranks they occupy.
fn mean_ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start;
        while end + 1 < order.len() && values[order[end + 1]] == values[order[start]] {
            end += 1;
        }
        let mean = (start + end) as f64 / 2.0 + 1.0;
        for position in start..=end {
            ranks[order[position]] = mean;
        }
        start = end + 1;
    }
    ranks
}

fn derived(state: &mut State, slot: usize) -> &mut Derived {
    if state.derived.len() <= slot {
        state.derived.resize_with(slot + 1, Derived::default);
    }
    &mut state.derived[slot]
}

fn count(length: i64) -> usize {
    usize::try_from(length).unwrap_or(0)
}

/// `na` for a missing or non-finite result (0/0 in a ratio), the value otherwise.
fn out(value: Option<f64>) -> RuntimeValue {
    value
        .filter(|value| value.is_finite())
        .map_or(RuntimeValue::Na, RuntimeValue::Float)
}

/// The last `length` values of the source, or None while the window is short or has an `na`.
fn window(inputs: &[RuntimeValue], length: usize) -> Option<Vec<f64>> {
    if length == 0 || length > inputs.len() {
        return None;
    }
    inputs[inputs.len() - length..]
        .iter()
        .map(RuntimeValue::number)
        .collect()
}

fn last_n(values: &[f64], length: usize) -> Option<&[f64]> {
    (length > 0 && length <= values.len()).then(|| &values[values.len() - length..])
}

/// `ta.alma(series, length, offset, sigma, floor)`: the Arnaud Legoux moving average.
pub fn alma(
    source: &RuntimeValue,
    length: i64,
    offset: f64,
    sigma: f64,
    floor: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let length = count(length);
    let Some(values) = window(&state.inputs, length) else {
        return RuntimeValue::Na;
    };
    let mut m = offset * (length as f64 - 1.0);
    if floor {
        m = m.floor();
    }
    let s = length as f64 / sigma;
    let (mut norm, mut sum) = (0.0, 0.0);
    for i in 0..length {
        let weight = (-((i as f64 - m).powi(2)) / (2.0 * s * s)).exp();
        norm += weight;
        sum += values[length - 1 - i] * weight;
    }
    out((norm != 0.0).then(|| sum / norm))
}

/// `ta.cog(source, length)`: the center of gravity oscillator.
pub fn cog(source: &RuntimeValue, length: i64, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let length = count(length);
    let Some(values) = window(&state.inputs, length) else {
        return RuntimeValue::Na;
    };
    let (mut weighted, mut total) = (0.0, 0.0);
    for i in 0..length {
        let value = values[length - 1 - i];
        weighted += value * (i as f64 + 1.0);
        total += value;
    }
    out((total != 0.0).then(|| -weighted / total))
}

/// `ta.mode(source, length)`: the most frequent value of the window, the smallest on a tie.
pub fn mode(source: &RuntimeValue, length: i64, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let Some(mut values) = window(&state.inputs, count(length)) else {
        return RuntimeValue::Na;
    };
    values.sort_by(f64::total_cmp);
    let (mut best, mut best_run) = (values[0], 0);
    let mut start = 0;
    while start < values.len() {
        let mut end = start;
        while end < values.len() && values[end] == values[start] {
            end += 1;
        }
        if end - start > best_run {
            best_run = end - start;
            best = values[start];
        }
        start = end;
    }
    out(Some(best))
}

/// `ta.percentile_nearest_rank` / `ta.percentile_linear_interpolation` over the last `length` values.
pub fn percentile(
    source: &RuntimeValue,
    length: i64,
    percentage: f64,
    interpolate: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let Some(mut values) = window(&state.inputs, count(length)) else {
        return RuntimeValue::Na;
    };
    if !(0.0..=100.0).contains(&percentage) {
        return RuntimeValue::Na;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if interpolate {
        let position = percentage / 100.0 * (n as f64 - 1.0);
        let below = position.floor() as usize;
        let above = position.ceil() as usize;
        out(Some(
            values[below] + (values[above] - values[below]) * (position - below as f64),
        ))
    } else {
        let rank = ((percentage / 100.0 * n as f64).ceil() as usize).clamp(1, n);
        out(Some(values[rank - 1]))
    }
}

/// `ta.valuewhen(condition, source, occurrence)`: the source on the bar the condition was last
/// true, `occurrence` true bars back (0 is the most recent).
pub fn valuewhen(
    condition: &RuntimeValue,
    source: &RuntimeValue,
    occurrence: i64,
    state: &mut State,
) -> RuntimeValue {
    if condition.bool() == Some(true) {
        state.seconds.push(source.clone());
    }
    let Ok(back) = usize::try_from(occurrence) else {
        return RuntimeValue::Na;
    };
    state
        .seconds
        .len()
        .checked_sub(back + 1)
        .map_or(RuntimeValue::Na, |index| state.seconds[index].clone())
}

/// `ta.mfi(series, length)`: the money flow index.
pub fn mfi(source: &RuntimeValue, length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    let previous = state.inputs.last().and_then(RuntimeValue::number);
    state.inputs.push(source.clone());
    let flow = match (source.number(), previous) {
        (Some(now), Some(before)) => {
            let up = if now - before <= 0.0 { 0.0 } else { now };
            let down = if now - before >= 0.0 { 0.0 } else { now };
            Some((bar.volume * up, bar.volume * down))
        }
        _ => None,
    };
    state
        .results
        .push(flow.map_or(RuntimeValue::Na, |(up, down)| {
            RuntimeValue::Tuple(vec![RuntimeValue::Float(up), RuntimeValue::Float(down)])
        }));
    let length = count(length);
    let Some(start) = state
        .results
        .len()
        .checked_sub(length)
        .filter(|_| length > 0)
    else {
        return RuntimeValue::Na;
    };
    let (mut upper, mut lower) = (0.0, 0.0);
    for entry in &state.results[start..] {
        let RuntimeValue::Tuple(pair) = entry else {
            return RuntimeValue::Na;
        };
        upper += pair[0].number().unwrap_or(0.0);
        lower += pair[1].number().unwrap_or(0.0);
    }
    out(Some(100.0 - 100.0 / (1.0 + upper / lower)))
}

/// `ta.vwma(source, length)`: the volume-weighted moving average.
pub fn vwma(source: &RuntimeValue, length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    state.seconds.push(RuntimeValue::Float(bar.volume));
    let length = count(length);
    let (Some(prices), Some(volumes)) = (
        window(&state.inputs, length),
        window(&state.seconds, length),
    ) else {
        return RuntimeValue::Na;
    };
    let weighted: f64 = prices.iter().zip(&volumes).map(|(p, v)| p * v).sum();
    let volume: f64 = volumes.iter().sum();
    out((volume != 0.0).then(|| weighted / volume))
}

/// The highest high and lowest low of the last `length` bars, from the bars this call site saw.
fn range(state: &State, length: usize) -> Option<(f64, f64)> {
    let highs = last_n(&state.highs, length)?;
    let lows = last_n(&state.lows, length)?;
    // A gap (`na`) anywhere in the window makes the range unknown, as in `ta.highest`.
    if highs.iter().chain(lows).any(|value| value.is_nan()) {
        return None;
    }
    Some((
        highs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        lows.iter().copied().fold(f64::INFINITY, f64::min),
    ))
}

/// `ta.stoch(source, high, low, length)`: where the source sits in the high-low range, 0 to 100.
pub fn stoch(
    source: &RuntimeValue,
    high: &RuntimeValue,
    low: &RuntimeValue,
    length: i64,
    state: &mut State,
) -> RuntimeValue {
    let (Some(source), Some(high), Some(low)) = (source.number(), high.number(), low.number())
    else {
        // A gap in the inputs poisons the window: keep the history aligned with `na` bars.
        state.highs.push(f64::NAN);
        state.lows.push(f64::NAN);
        return RuntimeValue::Na;
    };
    state.highs.push(high);
    state.lows.push(low);
    let Some((highest, lowest)) = range(state, count(length)) else {
        return RuntimeValue::Na;
    };
    out(
        (highest.is_finite() && lowest.is_finite() && highest != lowest)
            .then(|| 100.0 * (source - lowest) / (highest - lowest)),
    )
}

/// `ta.wpr(length)`: Williams %R, from the bar's own high, low and close.
pub fn wpr(length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    state.highs.push(bar.high);
    state.lows.push(bar.low);
    let Some((highest, lowest)) = range(state, count(length)) else {
        return RuntimeValue::Na;
    };
    out((highest != lowest).then(|| 100.0 * (bar.close - highest) / (highest - lowest)))
}

/// `ta.kc(series, length, mult, useTrueRange)` as `[basis, upper, lower]`.
pub fn keltner(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    use_true_range: bool,
    bar: &Kline,
    state: &mut State,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let length = count(length);
    let previous_close = state.closes.last().copied();
    state.closes.push(bar.close);
    let span = if use_true_range {
        ta::true_range(bar, previous_close)
    } else {
        bar.high - bar.low
    };
    let basis = derived(state, 0).ema(source.number(), length);
    let average_range = derived(state, 1).ema(Some(span), length);
    match (basis, average_range) {
        (Some(basis), Some(average)) => (
            Some(basis),
            Some(basis + average * multiplier),
            Some(basis - average * multiplier),
        ),
        _ => (None, None, None),
    }
}

/// `ta.kcw`: the Keltner channel's width relative to its basis.
pub fn keltner_width(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    use_true_range: bool,
    bar: &Kline,
    state: &mut State,
) -> RuntimeValue {
    let (basis, upper, lower) = keltner(source, length, multiplier, use_true_range, bar, state);
    match (basis, upper, lower) {
        (Some(basis), Some(upper), Some(lower)) if basis != 0.0 => {
            out(Some((upper - lower) / basis))
        }
        _ => RuntimeValue::Na,
    }
}

/// `ta.bbw(series, length, mult)`: the Bollinger band width relative to the basis.
pub fn bollinger_width(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    state: &mut State,
) -> RuntimeValue {
    let bands = ta::evaluate_bb(source, length, multiplier, state);
    let RuntimeValue::Tuple(parts) = bands else {
        return RuntimeValue::Na;
    };
    match (parts[0].number(), parts[1].number(), parts[2].number()) {
        (Some(basis), Some(upper), Some(lower)) if basis != 0.0 => {
            out(Some((upper - lower) / basis))
        }
        _ => RuntimeValue::Na,
    }
}

/// `ta.tsi(source, short_length, long_length)`: the true strength index, in -1 to 1.
pub fn tsi(source: &RuntimeValue, short: i64, long: i64, state: &mut State) -> RuntimeValue {
    let previous = state.inputs.last().and_then(RuntimeValue::number);
    state.inputs.push(source.clone());
    let change = source
        .number()
        .zip(previous)
        .map(|(now, before)| now - before);
    let (short, long) = (count(short), count(long));
    let momentum = derived(state, 0)
        .ema(change, long)
        .and_then(|v| derived(state, 1).ema(Some(v), short));
    let magnitude = derived(state, 2)
        .ema(change.map(f64::abs), long)
        .and_then(|v| derived(state, 3).ema(Some(v), short));
    match (momentum, magnitude) {
        (Some(m), Some(a)) if a != 0.0 => out(Some(m / a)),
        _ => RuntimeValue::Na,
    }
}

/// The cumulative volume variables a script reads without calling: `ta.obv`, `ta.accdist`, `ta.iii`,
/// `ta.nvi`, `ta.pvi`, `ta.pvt`, `ta.wad` and `ta.wvad`. `None` when `name` is not one of them.
///
/// `state` remembers how many bars it has folded in, so it catches up from the committed history
/// on a bar where the script did not read the variable, and a repeated read is free.
pub fn cumulative(
    name: &str,
    current: &Kline,
    runtime: &RuntimeState,
    state: &mut State,
) -> Option<RuntimeValue> {
    if !VARIABLES.contains(&name) {
        return None;
    }
    let series = |key: &str| -> Vec<f64> {
        runtime
            .histories
            .get(key)
            .map(|values| {
                values
                    .iter()
                    .map(|v| v.number().unwrap_or(f64::NAN))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (open, high, low, close, volume) = (
        series("open"),
        series("high"),
        series("low"),
        series("close"),
        series("volume"),
    );
    let bars = close.len();
    // `results` holds one value per folded bar; the bar being drawn is folded last and then dropped
    // again so it is recomputed once its close is final.
    let folded = state.results.len().min(bars);
    state.results.truncate(folded);
    for index in folded..=bars {
        let bar = if index < bars {
            Kline {
                open_time: 0,
                close_time: 0,
                open: open[index],
                high: high[index],
                low: low[index],
                close: close[index],
                volume: volume[index],
            }
        } else {
            current.clone()
        };
        let before = index.checked_sub(1).map(|i| Kline {
            open_time: 0,
            close_time: 0,
            open: open[i],
            high: high[i],
            low: low[i],
            close: close[i],
            volume: volume[i],
        });
        let prior = state.results.last().and_then(RuntimeValue::number);
        let next = fold(name, &bar, before.as_ref(), prior);
        if index < bars {
            state.results.push(out(Some(next)));
        } else {
            return Some(out(Some(next)));
        }
    }
    None
}

fn fold(name: &str, bar: &Kline, before: Option<&Kline>, prior: Option<f64>) -> f64 {
    let total = prior.unwrap_or(0.0);
    let span = bar.high - bar.low;
    let change = before.map(|b| bar.close - b.close);
    match name {
        "ta.obv" => {
            let direction = change.map_or(0.0, |c| match c {
                c if c > 0.0 => 1.0,
                c if c < 0.0 => -1.0,
                _ => 0.0,
            });
            total + direction * bar.volume
        }
        "ta.accdist" => {
            if (bar.close == bar.high && bar.close == bar.low) || span == 0.0 {
                total
            } else {
                total + (2.0 * bar.close - bar.low - bar.high) / span * bar.volume
            }
        }
        "ta.iii" => {
            let denominator = span * bar.volume;
            if denominator == 0.0 {
                total
            } else {
                total + (2.0 * bar.close - bar.high - bar.low) / denominator
            }
        }
        "ta.pvt" => match before {
            Some(b) if b.close != 0.0 => total + (bar.close - b.close) / b.close * bar.volume,
            _ => total,
        },
        "ta.nvi" | "ta.pvi" => {
            let level = prior.filter(|p| *p != 0.0).unwrap_or(1.0);
            let Some(b) = before.filter(|b| b.close != 0.0) else {
                return level;
            };
            let moves = if name == "ta.nvi" {
                bar.volume < b.volume
            } else {
                bar.volume > b.volume
            };
            if moves {
                level + (bar.close - b.close) / b.close * level
            } else {
                level
            }
        }
        "ta.wad" => {
            let Some(b) = before else { return total };
            let true_high = bar.high.max(b.close);
            let true_low = bar.low.min(b.close);
            let gain = match bar.close - b.close {
                m if m > 0.0 => bar.close - true_low,
                m if m < 0.0 => bar.close - true_high,
                _ => 0.0,
            };
            total + gain
        }
        // Not cumulative: the volume-weighted position of the close within the bar.
        _ => {
            if span == 0.0 {
                0.0
            } else {
                (bar.close - bar.open) / span * bar.volume
            }
        }
    }
}

#[cfg(test)]
mod pivot_and_rank_tests {
    use super::*;

    fn kline(open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    fn rank(values: &[f64], length: i64) -> RuntimeValue {
        let mut state = State::default();
        let mut last = RuntimeValue::Na;
        for value in values {
            last = rci(&RuntimeValue::Float(*value), length, &mut state);
        }
        last
    }

    fn close_to(actual: &RuntimeValue, expected: f64) {
        let value = actual.number().unwrap_or_else(|| panic!("{actual:?}"));
        assert!((value - expected).abs() < 1e-9, "{value} vs {expected}");
    }

    #[test]
    fn rci_is_100_for_a_rise_minus_100_for_a_fall_and_in_between_otherwise() {
        close_to(&rank(&[1.0, 2.0, 3.0, 4.0], 4), 100.0);
        close_to(&rank(&[9.0, 7.0, 4.0, 1.0], 4), -100.0);
        // Ranks 1, 3, 2, 4 against 1…4: 1 - 6·2 / (4·15) = 0.8.
        close_to(&rank(&[10.0, 30.0, 20.0, 40.0], 4), 80.0);
        // Only the last `length` values count.
        close_to(&rank(&[100.0, 1.0, 2.0, 3.0, 4.0], 4), 100.0);
    }

    #[test]
    fn rci_gives_equal_values_the_mean_of_their_ranks() {
        // Ranks 1.5, 1.5, 3, 4 against 1…4: 4.5 / sqrt(5 · 4.5).
        close_to(
            &rank(&[1.0, 1.0, 2.0, 3.0], 4),
            4.5 / 22.5_f64.sqrt() * 100.0,
        );
        assert_eq!(
            rank(&[5.0, 5.0, 5.0], 3),
            RuntimeValue::Na,
            "undefined when all are equal"
        );
    }

    #[test]
    fn rci_waits_for_a_full_window_without_gaps() {
        assert_eq!(rank(&[1.0, 2.0], 3), RuntimeValue::Na);
        assert_eq!(
            rank(&[1.0, 2.0, 3.0], 1),
            RuntimeValue::Na,
            "one value has no order"
        );
        let mut state = State::default();
        for value in [
            RuntimeValue::Float(1.0),
            RuntimeValue::Na,
            RuntimeValue::Float(3.0),
        ] {
            assert_eq!(rci(&value, 3, &mut state), RuntimeValue::Na);
        }
    }

    /// One period of two bars (O 100, H 110, L 90, C 100 after the second), then the anchor bar,
    /// whose open is 104.
    fn levels_after_a_period(kind: &str) -> [Option<f64>; 11] {
        let mut state = State::default();
        assert_eq!(
            pivot_point_levels(
                kind,
                false,
                false,
                &kline(100.0, 105.0, 95.0, 102.0),
                &mut state
            ),
            Some([None; 11]),
            "no period has ended yet"
        );
        pivot_point_levels(
            kind,
            false,
            false,
            &kline(102.0, 110.0, 90.0, 100.0),
            &mut state,
        );
        pivot_point_levels(
            kind,
            true,
            false,
            &kline(104.0, 106.0, 101.0, 105.0),
            &mut state,
        )
        .expect("a known type")
    }

    fn levels(kind: &str) -> Vec<(&'static str, f64)> {
        PIVOT_LEVELS
            .iter()
            .zip(levels_after_a_period(kind))
            .filter_map(|(name, value)| value.map(|value| (*name, value)))
            .collect()
    }

    fn assert_levels(kind: &str, expected: &[(&str, f64)]) {
        let actual = levels(kind);
        assert_eq!(actual.len(), expected.len(), "{kind}: {actual:?}");
        for ((name, value), (expected_name, expected_value)) in actual.iter().zip(expected) {
            assert_eq!(name, expected_name, "{kind}");
            assert!(
                (value - expected_value).abs() < 1e-9,
                "{kind} {name}: {value} vs {expected_value}"
            );
        }
    }

    #[test]
    fn traditional_levels_run_to_five_each_side() {
        assert_levels(
            "Traditional",
            &[
                ("P", 100.0),
                ("R1", 110.0),
                ("S1", 90.0),
                ("R2", 120.0),
                ("S2", 80.0),
                ("R3", 130.0),
                ("S3", 70.0),
                ("R4", 140.0),
                ("S4", 60.0),
                ("R5", 150.0),
                ("S5", 50.0),
            ],
        );
    }

    #[test]
    fn the_other_types_have_the_levels_their_formulas_give() {
        assert_levels(
            "Fibonacci",
            &[
                ("P", 100.0),
                ("R1", 107.64),
                ("S1", 92.36),
                ("R2", 112.36),
                ("S2", 87.64),
                ("R3", 120.0),
                ("S3", 80.0),
            ],
        );
        assert_levels(
            "Classic",
            &[
                ("P", 100.0),
                ("R1", 110.0),
                ("S1", 90.0),
                ("R2", 120.0),
                ("S2", 80.0),
                ("R3", 140.0),
                ("S3", 60.0),
                ("R4", 160.0),
                ("S4", 40.0),
            ],
        );
        // Open and close of the period are both 100: X = H + L + 2C = 400.
        assert_levels("DM", &[("P", 100.0), ("R1", 110.0), ("S1", 90.0)]);
        // Woodie weighs the open of the bar that sets the levels, here 104.
        assert_levels(
            "Woodie",
            &[
                ("P", 102.0),
                ("R1", 114.0),
                ("S1", 94.0),
                ("R2", 122.0),
                ("S2", 82.0),
                ("R3", 134.0),
                ("S3", 74.0),
                ("R4", 154.0),
                ("S4", 54.0),
            ],
        );
        let range = 20.0;
        assert_levels(
            "Camarilla",
            &[
                ("P", 100.0),
                ("R1", 100.0 + range * 1.1 / 12.0),
                ("S1", 100.0 - range * 1.1 / 12.0),
                ("R2", 100.0 + range * 1.1 / 6.0),
                ("S2", 100.0 - range * 1.1 / 6.0),
                ("R3", 100.0 + range * 1.1 / 4.0),
                ("S3", 100.0 - range * 1.1 / 4.0),
                ("R4", 100.0 + range * 1.1 / 2.0),
                ("S4", 100.0 - range * 1.1 / 2.0),
                ("R5", 110.0 / 90.0 * 100.0),
                ("S5", 100.0 - (110.0 / 90.0 * 100.0 - 100.0)),
            ],
        );
    }

    #[test]
    fn dm_pivots_lean_on_the_direction_of_the_period() {
        let mut up = State::default();
        pivot_point_levels(
            "DM",
            false,
            false,
            &kline(100.0, 110.0, 90.0, 108.0),
            &mut up,
        );
        let up =
            pivot_point_levels("DM", true, false, &kline(0.0, 0.0, 0.0, 0.0), &mut up).unwrap();
        // Close above open: X = 2H + L + C = 418.
        assert_eq!(up[0], Some(418.0 / 4.0));
        let mut down = State::default();
        pivot_point_levels(
            "DM",
            false,
            false,
            &kline(108.0, 110.0, 90.0, 100.0),
            &mut down,
        );
        let down =
            pivot_point_levels("DM", true, false, &kline(0.0, 0.0, 0.0, 0.0), &mut down).unwrap();
        // Close below open: X = H + 2L + C = 390.
        assert_eq!(down[0], Some(390.0 / 4.0));
    }

    #[test]
    fn levels_hold_between_anchors_and_developing_levels_follow_the_bars() {
        let mut state = State::default();
        let at = |state: &mut State, anchor, developing, bar: Kline| {
            pivot_point_levels("Traditional", anchor, developing, &bar, state).unwrap()[0]
        };
        assert_eq!(
            at(&mut state, true, false, kline(100.0, 110.0, 90.0, 100.0)),
            None
        );
        assert_eq!(
            at(&mut state, false, false, kline(100.0, 120.0, 80.0, 100.0)),
            None
        );
        // The anchor ends the period of those two bars: H 120, L 80, C 100.
        assert_eq!(
            at(&mut state, true, false, kline(100.0, 130.0, 70.0, 130.0)),
            Some(100.0)
        );
        // It holds while the new period runs …
        assert_eq!(
            at(&mut state, false, false, kline(130.0, 140.0, 125.0, 135.0)),
            Some(100.0)
        );
        // … but a developing pivot is the running period's own: bars (130,70,130) and (140,125,135).
        let developing = at(&mut state, false, true, kline(135.0, 136.0, 134.0, 135.0));
        assert_eq!(developing, Some((140.0 + 70.0 + 135.0) / 3.0));
    }

    #[test]
    fn an_unknown_type_has_no_levels() {
        let mut state = State::default();
        assert_eq!(
            pivot_point_levels(
                "Pythagorean",
                true,
                false,
                &kline(1.0, 2.0, 0.5, 1.5),
                &mut state
            ),
            None
        );
    }
}
