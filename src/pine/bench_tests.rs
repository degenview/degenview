//! Release-mode benchmarks for the Pine engine. Every test here is `#[ignore]`: run them with
//! `tools/pine-bench.sh` (which builds two commits and interleaves their runs) or directly:
//!
//! ```text
//! cargo test --release --lib pine::bench_tests -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Each workload prints one machine-readable line:
//! `BENCH <name> median_ns=… min_ns=… max_ns=… checksum=… allocs=… bytes=… peak=…`.
//! The checksum hashes what the script computed (plots, trades), so two builds that print different
//! checksums for a workload did not run the same program and their timings are not comparable.
//! Allocation columns are zero unless the `alloc-count` feature is on.
//!
//! `PINE_BENCH_REPS` (default 15) sets the timed repetitions per workload, `PINE_BENCH_ONLY` is a
//! comma-separated list of workload names to run.

use std::collections::HashMap;
use std::time::Instant;

use crate::alloc_count;
use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::execution::ExecutionController;
use crate::pine::program::{SymbolInfo, VisualOutput};
use crate::pine::runtime_support::{SecurityDataProvider, SecurityKey};
use crate::pine::session::Session;
use crate::pine::token::Limits;

const START_MS: i64 = 1_700_000_000_000;
pub(crate) const MINUTE_MS: i64 = 60_000;

/// A fixed-seed random walk, so every build runs the same data.
pub(crate) fn bars(count: usize, spacing_ms: i64) -> Vec<Kline> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut close = 100.0;
    (0..count)
        .map(|index| {
            let open = close;
            close = (open * (1.0 + (next() - 0.5) * 0.04)).max(1.0);
            Kline {
                open_time: START_MS + index as i64 * spacing_ms,
                close_time: START_MS + (index as i64 + 1) * spacing_ms,
                open,
                high: open.max(close) * (1.0 + next() * 0.01),
                low: open.min(close) * (1.0 - next() * 0.01),
                close,
                volume: 100.0 + next() * 900.0,
            }
        })
        .collect()
}

/// Serves the requested symbol as a walk at the requested interval.
struct WalkData {
    start_ms: i64,
    end_ms: i64,
}

impl SecurityDataProvider for WalkData {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        let interval_ms = (key.interval * 1000.0) as i64;
        if interval_ms <= 0 {
            return None;
        }
        let count = ((self.end_ms - self.start_ms) / interval_ms + 2) as usize;
        let mut walk = bars(count, interval_ms);
        for bar in &mut walk {
            bar.open_time -= START_MS - self.start_ms;
            bar.close_time -= START_MS - self.start_ms;
        }
        Some(walk)
    }
}

fn limits() -> Limits {
    Limits {
        deadline_seconds: 600.0,
        ..Limits::default()
    }
}

fn symbol() -> SymbolInfo {
    SymbolInfo {
        ticker: "BTCUSDT".to_owned(),
        ticker_id: "BINANCE:BTCUSDT".to_owned(),
        currency: "USD".to_owned(),
        kind: "crypto".to_owned(),
        ..SymbolInfo::default()
    }
}

fn mix(hash: &mut u64, bits: u64) {
    *hash ^= bits;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
}

fn checksum(output: &VisualOutput) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for plot in &output.plots {
        for value in &plot.values {
            mix(&mut hash, value.map_or(u64::MAX, f64::to_bits));
        }
    }
    if let Some(report) = &output.strategy {
        mix(&mut hash, report.trades.len() as u64);
        mix(&mut hash, report.open_trades.len() as u64);
    }
    mix(&mut hash, output.bar_count as u64);
    hash
}

fn compile(source: &str) -> crate::pine::program::CompiledProgram {
    let program = compiler::compile(source, limits(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    program
}

/// Runs `source` over `bars` on a session that `configure` may adjust first (a test attaches a
/// debugger there), and returns the session so its output can be read.
pub(crate) fn run_script(
    source: &str,
    bars: &[Kline],
    security: bool,
    configure: impl FnOnce(&mut Session),
) -> Session {
    let provider = security.then(|| {
        Box::new(WalkData {
            start_ms: bars[0].open_time,
            end_ms: bars[bars.len() - 1].close_time,
        }) as Box<dyn SecurityDataProvider>
    });
    let mut session = Session::new(
        compile(source),
        HashMap::new(),
        limits(),
        Some(0.01),
        symbol(),
        provider,
        None,
    );
    configure(&mut session);
    session.evaluate(bars).expect("script runs");
    session
}

fn evaluate(source: &str, bars: &[Kline], security: bool) -> u64 {
    checksum(&run_script(source, bars, security, |_| {}).output())
}

pub(crate) const SIMPLE: &str = "//@version=6
indicator(\"Simple\")
fast = ta.ema(close, 9)
slow = ta.ema(close, 21)
signal = fast > slow
plot(fast)
plot(slow)
plot(signal ? 1 : 0)
";

/// A long straight-line script: statements dominate, so the per-statement cost shows.
pub(crate) fn straight_line(statements: usize) -> String {
    let mut source = String::from("//@version=6\nindicator(\"Straight\")\nv0 = close\n");
    for index in 1..statements {
        source.push_str(&format!(
            "v{index} = v{} * 1.0001 + {}.5\n",
            index - 1,
            index % 7
        ));
    }
    source.push_str(&format!("plot(v{})\n", statements - 1));
    source
}

pub(crate) const MANY_TA: &str = "//@version=6
indicator(\"Many ta\")
a = ta.sma(close, 20)
b = ta.ema(close, 12)
c = ta.rsi(close, 14)
d = ta.stdev(close, 20)
e = ta.highest(high, 10)
f = ta.lowest(low, 10)
g = ta.change(close, 3)
h = ta.rma(close, 14)
i = ta.wma(close, 10)
j = ta.crossover(a, b)
k = ta.sma(a - b, 5)
l = ta.ema(c, 9)
plot(a + b + c + d + e + f + g + h + i + k + l + (j ? 1 : 0))
";

pub(crate) const LONG_HISTORY: &str = "//@version=6
indicator(\"History\")
s = close - close[1]
t = ta.sma(s, 50)
u = nz(t[100]) + nz(close[500])
plot(u)
";

pub(crate) const CONTAINERS: &str = "//@version=6
indicator(\"Containers\")
var arr = array.new_float(0)
var counts = map.new<string, float>()
var grid = matrix.new<float>(3, 3, 0.0)
array.push(arr, close)
if array.size(arr) > 50
    array.shift(arr)
total = 0.0
for i = 0 to array.size(arr) - 1
    total += array.get(arr, i)
map.put(counts, \"last\", close)
matrix.set(grid, 1, 1, close)
plot(total / array.size(arr) + map.get(counts, \"last\") + matrix.get(grid, 1, 1))
";

pub(crate) const NESTED_FUNCTIONS: &str = "//@version=6
indicator(\"Functions\")
leaf(x) =>
    y = x * 2
    y + 1
mid(x) =>
    a = leaf(x)
    b = leaf(a)
    a + b
top(x) =>
    mid(x) + mid(x + 1)
v = top(close)
plot(v)
";

pub(crate) const SECURITY: &str = "//@version=6
indicator(\"Security\")
htf = request.security(syminfo.tickerid, \"5\", ta.sma(close, 10))
plot(htf)
";

pub(crate) const STRATEGY: &str = "//@version=6
strategy(\"Cross\", overlay = true)
fast = ta.sma(close, 10)
slow = ta.sma(close, 30)
if ta.crossover(fast, slow)
    strategy.entry(\"L\", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close(\"L\")
plot(fast)
";

/// Several indicators executing at once, as a chart with scripts and an alert subscription does.
fn simultaneous(data: &[Kline]) -> u64 {
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| evaluate(SIMPLE, data, false)))
            .collect();
        workers.into_iter().fold(0u64, |sum, worker| {
            sum.wrapping_add(worker.join().expect("worker"))
        })
    })
}

/// A rebuild over history, then live ticks and closes through the controller's incremental path.
fn realtime(history: usize, steps: usize) -> u64 {
    let mut candles = bars(history + steps + 1, MINUTE_MS);
    let future = candles.split_off(history);
    let mut controller = ExecutionController::new(
        SIMPLE,
        HashMap::new(),
        symbol(),
        None,
        limits(),
        &NoLibraries,
        None,
        None,
    )
    .expect("valid script");
    let mut last = controller.update(&candles, None, candles[history - 1].close_time);
    for (step, next) in future.iter().enumerate().take(steps) {
        let open = START_MS + (history + step) as i64 * MINUTE_MS;
        let mut bar = next.clone();
        let mut snapshot = candles.clone();
        snapshot.push(bar.clone());
        controller.update(&snapshot, None, open + 10_000);
        bar.close *= 1.001;
        *snapshot.last_mut().expect("live bar") = bar.clone();
        controller.update(&snapshot, None, open + 20_000);
        last = controller.update(&snapshot, None, open + MINUTE_MS);
        candles.push(bar);
    }
    checksum(&last.output)
}

struct Workload {
    name: &'static str,
    run: Box<dyn Fn() -> u64>,
}

fn workloads() -> Vec<Workload> {
    let minute = bars(5_000, MINUTE_MS);
    let long = bars(20_000, MINUTE_MS);
    let straight = straight_line(800);
    let mut list = Vec::new();
    let mut add = |name: &'static str, run: Box<dyn Fn() -> u64>| list.push(Workload { name, run });
    {
        let data = minute.clone();
        add(
            "simple_indicator",
            Box::new(move || evaluate(SIMPLE, &data, false)),
        );
    }
    {
        let data = bars(800, MINUTE_MS);
        add(
            "straight_line_800",
            Box::new(move || evaluate(&straight, &data, false)),
        );
    }
    {
        let data = minute.clone();
        add("many_ta", Box::new(move || evaluate(MANY_TA, &data, false)));
    }
    {
        let data = long.clone();
        add(
            "long_history",
            Box::new(move || evaluate(LONG_HISTORY, &data, false)),
        );
    }
    {
        let data = minute.clone();
        add(
            "containers",
            Box::new(move || evaluate(CONTAINERS, &data, false)),
        );
    }
    {
        let data = minute.clone();
        add(
            "nested_functions",
            Box::new(move || evaluate(NESTED_FUNCTIONS, &data, false)),
        );
    }
    {
        let data = minute.clone();
        add(
            "security",
            Box::new(move || evaluate(SECURITY, &data, true)),
        );
    }
    {
        let data = minute.clone();
        add(
            "strategy",
            Box::new(move || evaluate(STRATEGY, &data, false)),
        );
    }
    {
        let data = minute.clone();
        add("simultaneous_x4", Box::new(move || simultaneous(&data)));
    }
    add("realtime_ticks", Box::new(|| realtime(2_000, 300)));
    {
        let source = straight_line(800);
        add(
            "compile_800",
            Box::new(move || {
                let program = compile(&source);
                program.statements.len() as u64
            }),
        );
    }
    list
}

fn percentile(sorted: &[u128], numerator: usize) -> u128 {
    sorted[(sorted.len() - 1) * numerator / 100]
}

#[test]
#[ignore = "benchmark: run with tools/pine-bench.sh"]
fn bench_workloads() {
    let repetitions: usize = std::env::var("PINE_BENCH_REPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let only: Option<Vec<String>> = std::env::var("PINE_BENCH_ONLY")
        .ok()
        .map(|value| value.split(',').map(str::to_owned).collect());
    for workload in workloads() {
        if only
            .as_ref()
            .is_some_and(|names| !names.iter().any(|name| name == workload.name))
        {
            continue;
        }
        // One untimed run warms caches and measures allocations, which do not vary run to run.
        alloc_count::reset_peak();
        let before = alloc_count::snapshot();
        let checksum = (workload.run)();
        let after = alloc_count::snapshot();
        let mut times: Vec<u128> = (0..repetitions)
            .map(|_| {
                let started = Instant::now();
                let again = (workload.run)();
                let elapsed = started.elapsed().as_nanos();
                assert_eq!(again, checksum, "{} is not deterministic", workload.name);
                elapsed
            })
            .collect();
        times.sort_unstable();
        println!(
            "BENCH {} median_ns={} min_ns={} max_ns={} checksum={checksum:016x} allocs={} bytes={} peak={}",
            workload.name,
            percentile(&times, 50),
            times[0],
            times[times.len() - 1],
            after.allocations - before.allocations,
            after.bytes - before.bytes,
            after.peak,
        );
    }
}
