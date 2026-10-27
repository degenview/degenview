//! Runs cached community TradingView scripts through the engine and diffs the outcome against
//! `src/pine/corpus/expectations.json`. A port of the Swift `PineCorpusTests`.
//!
//! The sources are third-party and never committed: `tools/pine-corpus/fetch.py` caches them in
//! `.pine-corpus/` (gitignored), and the test skips when that cache is missing. Point at another
//! location with `PINE_CORPUS_DIR=/path`; run just a few scripts with `PINE_CORPUS_ONLY=slug,slug`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::Kline;
use crate::pine::compiler::{self, LibraryResolver};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_support::{SecurityDataProvider, SecurityKey};
use crate::pine::session::{RunError, Session};
use crate::pine::token::{Diagnostic, DiagnosticSeverity, Limits};

#[derive(Deserialize)]
struct ManifestEntry {
    slug: String,
    name: String,
    kind: String,
    status: String,
}

#[derive(Clone, Deserialize, PartialEq)]
struct Expectation {
    #[serde(default = "default_ok")]
    status: String,
    #[serde(default)]
    codes: Vec<String>,
}

fn default_ok() -> String {
    "ok".to_owned()
}

/// Resolves `import` from the libraries `fetch.py --imports` cached beside the scripts.
struct CachedLibraries {
    directory: PathBuf,
}

impl LibraryResolver for CachedLibraries {
    fn source_for_library(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.directory.join(format!("{path}.pine"))).ok()
    }
}

/// A deterministic random walk per symbol, like a data feed with more history than the chart shows.
struct SyntheticSecurityData {
    range_start_ms: i64,
    range_end_ms: i64,
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl SecurityDataProvider for SyntheticSecurityData {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        const HISTORY: i64 = 500;
        let interval_ms = (key.interval * 1000.0) as i64;
        if interval_ms <= 0 {
            return None;
        }
        let mut state = fnv(key.symbol.as_bytes());
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut open_time = self.range_start_ms - HISTORY * interval_ms;
        let mut close = 100.0;
        let mut bars = Vec::new();
        while open_time <= self.range_end_ms {
            let open = close;
            close = (open * (1.0 + (next() - 0.5) * 0.04)).max(1.0);
            bars.push(Kline {
                open_time,
                close_time: open_time + interval_ms,
                open,
                high: open.max(close) * (1.0 + next() * 0.01),
                low: open.min(close) * (1.0 - next() * 0.01),
                close,
                volume: 100.0 + next() * 900.0,
            });
            open_time += interval_ms;
        }
        Some(bars)
    }
}

/// A deterministic random walk (fixed-seed LCG), so a failure reproduces on every machine.
fn synthetic_bars(count: usize) -> Vec<Kline> {
    // `PINE_CORPUS_SPACING` (seconds, default 60) picks the chart timeframe: scripts keyed to the
    // day or the session only draw anything once the history spans several of them.
    let spacing_ms: i64 = std::env::var("PINE_CORPUS_SPACING")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .map_or(60_000, |seconds| seconds * 1_000);
    const START_MS: i64 = 1_700_000_000_000;
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

/// What one script did: the outcome class, the distinct error codes, and one line per error with
/// its position, so a mismatch explains itself without a second run.
struct Outcome {
    status: String,
    codes: Vec<String>,
    details: Vec<String>,
    warnings: usize,
    /// What the run drew, for `PINE_CORPUS_REPORT`: a script that runs but draws nothing is suspect.
    drawn: String,
}

impl Outcome {
    fn new(status: &str, errors: &[&Diagnostic], warnings: usize) -> Self {
        let mut codes: Vec<String> = errors.iter().map(|d| d.code.clone()).collect();
        codes.sort();
        codes.dedup();
        let details = errors
            .iter()
            .take(5)
            .map(|d| {
                format!(
                    "{} {}:{} {}",
                    d.code, d.range.start.line, d.range.start.column, d.message
                )
            })
            .collect();
        Self {
            status: status.to_owned(),
            codes,
            details,
            warnings,
            drawn: String::new(),
        }
    }

    fn ok(warnings: usize) -> Self {
        Self::new("ok", &[], warnings)
    }
}

/// The corpus checks compatibility, not speed (`PINE_CORPUS_REPORT` prints the time per script), so
/// a slow debug build gets a long deadline instead of failing a script that merely runs slowly.
fn corpus_limits() -> Limits {
    Limits {
        deadline_seconds: 120.0,
        ..Limits::default()
    }
}

/// Compiles `source` and, when it is valid, runs it over `bars` with default inputs.
/// A session for `program` the way the corpus runs it.
fn corpus_session(program: crate::pine::program::CompiledProgram, bars: &[Kline]) -> Session {
    let symbol = SymbolInfo {
        ticker: "BTCUSDT".to_owned(),
        ticker_id: "BINANCE:BTCUSDT".to_owned(),
        currency: "USD".to_owned(),
        kind: "crypto".to_owned(),
        base_currency: None,
        ..SymbolInfo::default()
    };
    let security = SyntheticSecurityData {
        range_start_ms: bars[0].open_time,
        range_end_ms: bars.last().map_or(bars[0].open_time, |bar| bar.open_time),
    };
    Session::new(
        program,
        HashMap::new(),
        corpus_limits(),
        // The synthetic prices have many decimals; real symbols trade on a tick like this one.
        Some(0.01),
        symbol,
        Some(Box::new(security)),
        None,
    )
}

fn run(source: &str, bars: &[Kline], libraries: &dyn LibraryResolver) -> Outcome {
    let program = compiler::compile(source, corpus_limits(), libraries);
    let compile_errors: Vec<&Diagnostic> = program
        .diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    let warnings = program.diagnostics.len() - compile_errors.len();
    if std::env::var_os("PINE_CORPUS_WARNINGS").is_some() {
        for diagnostic in &program.diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Warning {
                eprintln!("WARNING {} {}", diagnostic.code, diagnostic.message);
            }
        }
    }
    if !compile_errors.is_empty() {
        return Outcome::new("compile-error", &compile_errors, warnings);
    }
    if program.statements.is_empty() {
        return Outcome::ok(warnings);
    }
    let mut session = corpus_session(program, bars);
    // `PINE_CORPUS_DUAL=1`: run the script again with every call on the chain and require the same
    // output, which checks the routing of builtin calls (`call_binding`) on real scripts.
    // A script that shows the wall clock (`timenow`) differs between any two runs.
    let chained = std::env::var_os("PINE_CORPUS_DUAL")
        .filter(|_| !source.contains("timenow"))
        .map(|_| {
            let mut other =
                corpus_session(compiler::compile(source, corpus_limits(), libraries), bars);
            other.ignore_routes = true;
            other
                .evaluate(bars)
                .map(|result| format!("{:?}", result.output))
        });
    match session.evaluate(bars) {
        Ok(result) => {
            let output = &result.output;
            if let Some(Ok(chained)) = &chained {
                assert_eq!(
                    &format!("{output:?}"),
                    chained,
                    "routed and chained runs differ"
                );
            }
            let plotted: usize = output
                .plots
                .iter()
                .map(|plot| plot.values.iter().flatten().count())
                .sum();
            let mut outcome = Outcome::ok(warnings);
            outcome.drawn = format!(
                "plots={}/{plotted} markers={} bg={} fills={} lines={} labels={} boxes={} tables={} alerts={} trades={}",
                output.plots.len(),
                output.markers.len(),
                output.backgrounds.len(),
                output.fills.len(),
                output.lines.len(),
                output.labels.len(),
                output.boxes.len(),
                output.tables.len(),
                output.alerts.len(),
                output
                    .strategy
                    .as_ref()
                    .map_or(0, |report| report.trades.len() + report.open_trades.len()),
            );
            outcome
        }
        Err(RunError::Script(diagnostic)) => {
            Outcome::new("runtime-error", &[&diagnostic], warnings)
        }
        Err(RunError::Rejection(_)) => Outcome::ok(warnings),
    }
}

/// Diffs one script against `expectations.json` (missing means "ok, no codes"). `Err` carries the
/// whole explanation; with `PINE_CORPUS_REPORT=1` every script is also printed.
fn check(
    label: &str,
    slug: &str,
    outcome: &Outcome,
    expectations: &HashMap<String, Expectation>,
) -> Result<(), String> {
    if std::env::var_os("PINE_CORPUS_REPORT").is_some() {
        eprintln!(
            "{slug:<10} {:<14} warnings={:<3} {} {}",
            outcome.status,
            outcome.warnings,
            outcome.drawn,
            outcome.details.join(" | ")
        );
    }
    let expected = expectations.get(slug).cloned().unwrap_or(Expectation {
        status: "ok".to_owned(),
        codes: Vec::new(),
    });
    if outcome.status == expected.status && outcome.codes == expected.codes {
        return Ok(());
    }
    Err(format!(
        "{slug} ({label}): expected {} {:?}, got {} {:?}\n    {}",
        expected.status,
        expected.codes,
        outcome.status,
        outcome.codes,
        outcome.details.join("\n    ")
    ))
}

#[test]
fn corpus_matches_expectations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus_dir = root.join("src/pine/corpus");
    let cache_dir = std::env::var_os("PINE_CORPUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".pine-corpus"));
    let expectations: HashMap<String, Expectation> = serde_json::from_str(
        &std::fs::read_to_string(corpus_dir.join("expectations.json")).unwrap(),
    )
    .unwrap();
    let only: Option<HashSet<String>> = std::env::var("PINE_CORPUS_ONLY")
        .ok()
        .map(|value| value.split(',').map(str::to_owned).collect());
    let bars = synthetic_bars(
        std::env::var("PINE_CORPUS_BARS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1_000),
    );
    let mut mismatches = Vec::new();
    let libraries = CachedLibraries {
        directory: cache_dir.join("imports"),
    };

    // Committed original scripts always run, so the harness diffs non-trivial output without the
    // network. `scripts/<kind>/<slug>.pine`; the slug is the file stem.
    let scripts_dir = corpus_dir.join("scripts");
    for kind in ["indicator", "strategy", "library"] {
        let Ok(entries) = std::fs::read_dir(scripts_dir.join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("pine") {
                continue;
            }
            let Some(slug) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if only.as_ref().is_some_and(|set| !set.contains(&slug)) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let outcome = run(&source, &bars, &libraries);
            mismatches.extend(check("committed", &slug, &outcome, &expectations).err());
        }
    }

    // Community scripts, fetched by `tools/pine-corpus/fetch.py` into the gitignored cache.
    if !cache_dir.is_dir() {
        eprintln!(
            "skipping community corpus: no cache at {} (run tools/pine-corpus/fetch.py)",
            cache_dir.display()
        );
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
        return;
    }
    let manifest: Vec<ManifestEntry> =
        serde_json::from_str(&std::fs::read_to_string(corpus_dir.join("manifest.json")).unwrap())
            .unwrap();
    let fetched: Vec<&ManifestEntry> = manifest
        .iter()
        .filter(|entry| {
            entry.status == "fetched" && only.as_ref().is_none_or(|set| set.contains(&entry.slug))
        })
        .collect();
    for entry in &fetched {
        let path = cache_dir
            .join(&entry.kind)
            .join(format!("{}.pine", entry.slug));
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let started = std::time::Instant::now();
        let outcome = run(&source, &bars, &libraries);
        if std::env::var_os("PINE_CORPUS_REPORT").is_some() {
            eprintln!("  {} took {:.2?}", entry.slug, started.elapsed());
        }
        mismatches.extend(check(&entry.name, &entry.slug, &outcome, &expectations).err());
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} corpus scripts differ from expectations.json:\n{}",
        mismatches.len(),
        fetched.len(),
        mismatches.join("\n")
    );
    eprintln!("Pine corpus: {} scripts checked", fetched.len());
}

/// Runs one file and prints its outcome and time: `PINE_RUN_FILE=path [PINE_RUN_BARS=n] cargo test --release run_one_file -- --nocapture`.
/// A scratch tool for reducing a corpus failure; it does nothing without the variable.
#[test]
fn run_one_file() {
    let Some(path) = std::env::var_os("PINE_RUN_FILE") else {
        return;
    };
    let source = std::fs::read_to_string(&path).unwrap();
    let libraries = CachedLibraries {
        directory: Path::new(env!("CARGO_MANIFEST_DIR")).join(".pine-corpus/imports"),
    };
    let count = std::env::var("PINE_RUN_BARS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    let started = std::time::Instant::now();
    let outcome = run(&source, &synthetic_bars(count), &libraries);
    eprintln!(
        "{count} bars in {:.2?}: {} {:?} warnings={}\n{}",
        started.elapsed(),
        outcome.status,
        outcome.codes,
        outcome.warnings,
        outcome.details.join("\n")
    );
}
