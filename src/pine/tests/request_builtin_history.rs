//! `ta.tr[1]`, `hlcc4[1]` and `dayofweek[1]` read inside `request.security` against TradingView's own
//! numbers: a builtin the engine computes rather than stores has a history on the requested series
//! too, and it is that series' own.

use std::collections::HashMap;

use super::bench::{limits, symbol};
use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::runtime::session::Session;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};

const FIVE_MINUTES: i64 = 300_000;
const FIFTEEN_MINUTES: i64 = 900_000;
const HOUR: i64 = 3_600_000;

/// The candles of each bar length, in seconds.
struct Candles(HashMap<i64, Vec<Kline>>);

impl SecurityDataProvider for Candles {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        self.0.get(&(key.interval as i64)).cloned()
    }
}

fn number(text: &str) -> f64 {
    if text == "NaN" {
        f64::NAN
    } else {
        text.parse().expect("a number")
    }
}

/// `time open high low close volume ...` as a candle of `length`; the rest is whatever the probe
/// logged next to it.
fn logged_candle(parts: &[&str], length: i64) -> (Kline, Vec<f64>) {
    let time: i64 = parts[0].parse().expect("a time");
    let values: Vec<f64> = parts[1..].iter().map(|part| number(part)).collect();
    let candle = Kline {
        open_time: time,
        close_time: time + length - 1,
        open: values[0],
        high: values[1],
        low: values[2],
        close: values[3],
        volume: values[4],
    };
    (candle, values[5..].to_vec())
}

/// The chart's twelve readings, in the order the probe logged them: the chart's own `ta.tr[1]`,
/// `hlcc4[1]`, `dayofweek[1]`, then the same three inside a request for the chart's own 5 minutes,
/// 15 minutes and 1 hour.
const COLUMNS: [&str; 12] = [
    "chart ta.tr[1]",
    "chart hlcc4[1]",
    "chart dayofweek[1]",
    "5m ta.tr[1]",
    "5m hlcc4[1]",
    "5m dayofweek[1]",
    "15m ta.tr[1]",
    "15m hlcc4[1]",
    "15m dayofweek[1]",
    "1h ta.tr[1]",
    "1h hlcc4[1]",
    "1h dayofweek[1]",
];

/// TradingView's own output (BINANCE:BTCUSDT 5m, 259 bars from 2026-11-24 17:50 UTC, which cross
/// midnight): `R` lines are a chart bar (time, open, high, low, close, volume) and the twelve
/// `COLUMNS`; `H15` and `H60` lines are each 15 minute and 1 hour candle the requests were given,
/// which is all the data the engine is given for them. A request's value is the one for its own
/// last bar, so the first readings of a request, which TradingView made with candles from before
/// the log, are left out until the request has three candles that start within the log: `ta.tr[1]`
/// of the second candle is its range alone.
#[test]
fn builtin_histories_inside_a_request_give_what_tradingview_gives() {
    let text = include_str!("data/tradingview-request-builtins.log");
    let mut chart = Vec::new();
    let mut readings = Vec::new();
    let mut quarters = Vec::new();
    let mut hours = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts[0] {
            // `R bar time ...`: the bar number is not a candle value.
            "R" => {
                let (candle, values) = logged_candle(&parts[2..], FIVE_MINUTES);
                chart.push(candle);
                readings.push(values);
            }
            "H15" => quarters.push(logged_candle(&parts[1..], FIFTEEN_MINUTES).0),
            "H60" => hours.push(logged_candle(&parts[1..], HOUR).0),
            other => panic!("a line of {other}"),
        }
    }
    assert_eq!(chart.len(), 259);

    let source = "//@version=6\nindicator(\"Request builtins\")\n\
        plot(ta.tr[1])\nplot(hlcc4[1])\nplot(dayofweek[1])\n\
        plot(request.security(syminfo.tickerid, \"5\", ta.tr[1]))\n\
        plot(request.security(syminfo.tickerid, \"5\", hlcc4[1]))\n\
        plot(request.security(syminfo.tickerid, \"5\", dayofweek[1]))\n\
        plot(request.security(syminfo.tickerid, \"15\", ta.tr[1]))\n\
        plot(request.security(syminfo.tickerid, \"15\", hlcc4[1]))\n\
        plot(request.security(syminfo.tickerid, \"15\", dayofweek[1]))\n\
        plot(request.security(syminfo.tickerid, \"60\", ta.tr[1]))\n\
        plot(request.security(syminfo.tickerid, \"60\", hlcc4[1]))\n\
        plot(request.security(syminfo.tickerid, \"60\", dayofweek[1]))\n";
    let program = compiler::compile(source, limits(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let provider = Candles(HashMap::from([
        (300, chart.clone()),
        (900, quarters.clone()),
        (3600, hours.clone()),
    ]));
    let mut session = Session::new(
        program,
        HashMap::new(),
        limits(),
        Some(0.01),
        symbol(),
        Some(Box::new(provider)),
        None,
    );
    session.evaluate(&chart).expect("script runs");
    let output = session.output();
    assert_eq!(output.plots.len(), COLUMNS.len());

    // How many candles a request has finished by the end of chart bar `bar`, of those that start
    // within the logged bars: the engine builds a candle that starts before them from them alone.
    let finished = |candles: &[Kline], bar: usize| {
        candles
            .iter()
            .filter(|candle| {
                candle.open_time >= chart[0].open_time && candle.close_time <= chart[bar].close_time
            })
            .count()
    };
    let mut compared = 0;
    let mut mismatches = Vec::new();
    for (bar, wanted) in readings.iter().enumerate() {
        for (column, name) in COLUMNS.iter().enumerate() {
            let candles = match column {
                0..=5 => bar + 1,
                6..=8 => finished(&quarters, bar),
                _ => finished(&hours, bar),
            };
            if candles < 3 {
                continue;
            }
            compared += 1;
            let wanted = wanted[column];
            let ok = match output.plots[column].values[bar] {
                None => wanted.is_nan(),
                // TradingView prints eight decimals.
                Some(got) => (got - wanted).abs() <= 1e-8 * wanted.abs().max(1.0) + 6e-9,
            };
            if !ok {
                mismatches.push(format!(
                    "bar {bar} {name}: {:?} against TradingView's {wanted}",
                    output.plots[column].values[bar]
                ));
            }
        }
    }
    assert!(compared > 2_500, "{compared} readings compared");
    assert!(
        mismatches.is_empty(),
        "{} readings differ, first: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(8)]
    );
}

const DAY: i64 = 86_400_000;
const MINUTE: i64 = 60_000;
/// The first chart bar of `tradingview-request-builtins-obv.log`: its number and its open.
const FIRST_BAR: usize = 6561;
const FIRST_BAR_OPEN: i64 = 1_795_545_900_000;

/// `bars` folded into whole candles of `length`. A candle some of whose bars are not there (the
/// one the first bar is in the middle of) is left out.
fn folded(bars: &[Kline], length: i64) -> Vec<Kline> {
    let per = (length / FIVE_MINUTES) as usize;
    let start = |bar: &Kline| bar.open_time.div_euclid(length) * length;
    let mut candles = Vec::new();
    for group in bars.chunk_by(|a, b| start(a) == start(b)) {
        if group.len() != per {
            continue;
        }
        candles.push(Kline {
            open_time: start(&group[0]),
            close_time: start(&group[0]) + length - 1,
            open: group[0].open,
            high: group.iter().map(|bar| bar.high).fold(f64::MIN, f64::max),
            low: group.iter().map(|bar| bar.low).fold(f64::MAX, f64::min),
            close: group[per - 1].close,
            volume: group.iter().map(|bar| bar.volume).sum(),
        });
    }
    candles
}

/// The plots of `source` over `chart`, with the candles of other bar lengths (in seconds) `candles`.
fn plots_of(
    source: &str,
    chart: &[Kline],
    candles: HashMap<i64, Vec<Kline>>,
) -> Vec<Vec<Option<f64>>> {
    let program = compiler::compile(source, limits(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        limits(),
        Some(0.01),
        symbol(),
        Some(Box::new(Candles(candles))),
        None,
    );
    session.evaluate(chart).expect("script runs");
    session
        .output()
        .plots
        .iter()
        .map(|plot| plot.values.clone())
        .collect()
}

/// `ta.obv` and `time_tradingday` read with `[]` inside requests, and a request for the 1 minute
/// candles of each chart bar, against TradingView's own output (BINANCE:BTCUSDT 5m, 330 bars from
/// 2026-11-24 18:45 UTC, which cross midnight). `R` lines are a chart bar (open, high, low, close,
/// volume), then `ta.obv[1] - ta.obv[2]` on the chart, in a 15 minute and in a 1 hour request (the
/// step, because an OBV starts wherever its series does), then how many days before the chart bar's
/// own `time_tradingday[1]` was on the chart, in a 15 minute and in a 1 hour request. A request for
/// the chart's own 5 minutes read exactly what the chart did on every bar, so it is not logged. `L`
/// lines are the 1 minute candles of the last 24 bars (open, high, low, close, volume), then
/// `ta.tr[1]` and the OBV step read in `request.security_lower_tf` on each. The requests are given
/// the candles of the log folded to 15 minutes and 1 hour; the first readings, which TradingView
/// made with candles from before the log, are left out until a series has four candles of its own,
/// and so is the last bar, which was the chart's live one.
#[test]
fn obv_tradingday_and_lower_timeframe_histories_give_what_tradingview_gives() {
    let text = include_str!("data/tradingview-request-builtins-obv.log");
    struct Reading {
        steps: [f64; 3],
        days: [i64; 3],
    }
    let mut chart = Vec::new();
    let mut readings = Vec::new();
    // For each chart bar that has them: the 1 minute candles and, for each, `ta.tr[1]` and the OBV step.
    let mut intrabars: HashMap<usize, Vec<(Kline, f64, f64)>> = HashMap::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let bar: usize = parts[1].parse().expect("a bar");
        let values: Vec<f64> = parts[2..].iter().map(|part| number(part)).collect();
        let open = FIRST_BAR_OPEN + (bar - FIRST_BAR) as i64 * FIVE_MINUTES;
        let candle = |length: i64, open: i64| Kline {
            open_time: open,
            close_time: open + length - 1,
            open: values[0],
            high: values[1],
            low: values[2],
            close: values[3],
            volume: values[4],
        };
        match parts[0] {
            "R" => {
                chart.push(candle(FIVE_MINUTES, open));
                readings.push(Reading {
                    steps: [values[5], values[6], values[7]],
                    days: [values[8] as i64, values[9] as i64, values[10] as i64],
                });
            }
            "L" => {
                let seen = intrabars.entry(bar).or_default();
                let at = open + seen.len() as i64 * MINUTE;
                seen.push((candle(MINUTE, at), values[5], values[6]));
            }
            other => panic!("a line of {other}"),
        }
    }
    assert_eq!(chart.len(), 330);
    let first_with_intrabars = *intrabars.keys().min().expect("intrabars");
    let quarters = folded(&chart, FIFTEEN_MINUTES);
    let hours = folded(&chart, HOUR);
    let minutes: Vec<Kline> = {
        let mut bars: Vec<_> = intrabars.iter().collect();
        bars.sort_by_key(|(bar, _)| **bar);
        bars.into_iter()
            .flat_map(|(_, seen)| seen.iter().map(|(candle, _, _)| candle.clone()))
            .collect()
    };

    let step = "ta.obv[1] - ta.obv[2]";
    let mut source = String::from("//@version=6\nindicator(\"Request builtins\")\n");
    source.push_str(&format!("plot({step})\nplot(time_tradingday[1])\n"));
    for interval in ["5", "15", "60"] {
        for read in [step, "time_tradingday[1]"] {
            source.push_str(&format!(
                "plot(request.security(syminfo.tickerid, \"{interval}\", {read}))\n"
            ));
        }
    }
    source.push_str(&format!(
        "tr = request.security_lower_tf(syminfo.tickerid, \"1\", ta.tr[1])\n\
         obv = request.security_lower_tf(syminfo.tickerid, \"1\", {step})\n"
    ));
    for name in ["tr", "obv"] {
        for index in 0..5 {
            source.push_str(&format!(
                "plot(array.size({name}) > {index} ? array.get({name}, {index}) : na)\n"
            ));
        }
    }
    let plots = plots_of(
        &source,
        &chart,
        HashMap::from([
            (300, chart.clone()),
            (900, quarters.clone()),
            (3600, hours.clone()),
            (60, minutes),
        ]),
    );
    assert_eq!(plots.len(), 18);

    // How many candles a request has finished by the end of chart bar `bar`, of those that start
    // within the logged bars.
    let finished = |candles: &[Kline], bar: usize| {
        candles
            .iter()
            .filter(|candle| candle.close_time <= chart[bar].close_time)
            .count()
    };
    let close = |got: Option<f64>, wanted: f64, tolerance: f64| match got {
        None => wanted.is_nan(),
        Some(got) => (got - wanted).abs() <= tolerance,
    };
    let mut compared = 0;
    let mut mismatches = Vec::new();
    let mut note = |what: String, bar: usize, got: Option<f64>, wanted: f64, ok: bool| {
        if !ok {
            mismatches.push(format!(
                "bar {} {what}: {got:?} against TradingView's {wanted}",
                FIRST_BAR + bar
            ));
        }
    };
    // Not the last bar: it was the chart's live bar when logged, where a request returns the candle
    // it is still building and not the last one it finished.
    for (bar, reading) in readings.iter().enumerate().take(readings.len() - 1) {
        let day = chart[bar].open_time.div_euclid(DAY) * DAY;
        // The chart and a request for its own timeframe: from the third bar.
        let counts = [
            bar + 1,
            bar + 1,
            finished(&quarters, bar),
            finished(&hours, bar),
        ];
        let series = [
            ("chart", 0, 1, counts[0]),
            ("5m", 2, 3, counts[1]),
            ("15m", 4, 5, counts[2]),
            ("1h", 6, 7, counts[3]),
        ];
        for (index, (name, step_plot, day_plot, candles)) in series.iter().enumerate() {
            if *candles < 4 {
                continue;
            }
            // The chart's own readings stand for the 5 minute request's too.
            let which = index.saturating_sub(1);
            let wanted_step = reading.steps[which];
            let got = plots[*step_plot][bar];
            compared += 1;
            note(
                format!("{name} OBV step"),
                bar,
                got,
                wanted_step,
                close(got, wanted_step, 1e-8 * wanted_step.abs().max(1.0) + 6e-9),
            );
            let wanted_day = (day + reading.days[which] * DAY) as f64;
            let got = plots[*day_plot][bar];
            compared += 1;
            note(
                format!("{name} time_tradingday[1]"),
                bar,
                got,
                wanted_day,
                close(got, wanted_day, 0.5),
            );
        }
        // The 1 minute candles of the bar, past the first bar that has them.
        let Some(seen) = intrabars.get(&(FIRST_BAR + bar)) else {
            continue;
        };
        if FIRST_BAR + bar == first_with_intrabars {
            continue;
        }
        for (index, (_, trange, obv_step)) in seen.iter().enumerate() {
            for (plot, wanted) in [(8 + index, *trange), (13 + index, *obv_step)] {
                let got = plots[plot][bar];
                compared += 1;
                note(
                    format!("1 minute candle {index} of plot {plot}"),
                    bar,
                    got,
                    wanted,
                    close(got, wanted, 1e-8 * wanted.abs().max(1.0) + 6e-9),
                );
            }
        }
    }
    assert!(compared > 1_500, "{compared} readings compared");
    assert!(
        mismatches.is_empty(),
        "{} readings differ, first: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(8)]
    );
}
