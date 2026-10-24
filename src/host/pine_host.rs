//! Minimal Pine runtime host: compiles a script and runs it over a series of bars, returning the
//! chart `VisualOutput` the renderer consumes. Input resolution and library imports are wired
//! incrementally; the session itself owns the per-bar state machine.
#![allow(dead_code)] // transient until the chart wires this

use crate::model::synthetic::ticker;
use crate::model::{DataSource, Kline};
use crate::pine::compiler;
use crate::pine::program::{InputValue, SymbolInfo, VisualOutput};
use crate::pine::runtime_support::{ChartSpan, SecurityDataProvider, SecurityKey};
use crate::pine::session::{RunError, Session};
use crate::pine::token::{Diagnostic, DiagnosticSeverity, Limits};
use crate::script_manager::registry::LibraryRegistry;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fetches `request.security` series through the REST kline service, so a script can read a symbol
/// the chart does not carry. `candles` is blocking: one synchronous request per distinct key, which
/// the session caches for the rest of the run.
pub struct KlineSecurityDataProvider;

/// ~30 s TTL so repeated runs do not re-fetch a foreign symbol each bar.
const SECURITY_CACHE_TTL_MS: i64 = 30_000;
type SecurityCache = HashMap<(SecurityKey, Option<ChartSpan>), (Vec<Kline>, i64)>;
static SECURITY_CACHE: LazyLock<Mutex<SecurityCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The finest candle sizes the intrabar series are built from, coarsest first: a series of
/// `request.security_lower_tf` is folded from the coarsest of these that divides its length (12
/// minutes from 1-minute candles, 4 hours from 1-hour ones).
const INTRABAR_BASES: [(i64, &str); 6] = [
    (86_400, "1d"),
    (3_600, "1h"),
    (1_800, "30m"),
    (900, "15m"),
    (300, "5m"),
    (60, "1m"),
];

/// The most base candles fetched for one intrabar series; the newest are kept, so a long chart gets
/// intrabars for its recent part.
const MAX_INTRABAR_CANDLES: usize = 60_000;

impl KlineSecurityDataProvider {
    fn cached(key: &(SecurityKey, Option<ChartSpan>)) -> Option<Vec<Kline>> {
        let cache = SECURITY_CACHE.lock().unwrap();
        let (candles, fetched_at) = cache.get(key)?;
        (now_millis() - fetched_at < SECURITY_CACHE_TTL_MS).then(|| candles.clone())
    }

    fn remember(key: (SecurityKey, Option<ChartSpan>), candles: &[Kline]) {
        SECURITY_CACHE
            .lock()
            .unwrap()
            .insert(key, (candles.to_vec(), now_millis()));
    }

    /// The series of finer bars over the chart's whole span (or its `calc_bars_count` newest bars).
    fn intrabars(key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
        let interval = key.interval.round() as i64;
        let (base_seconds, token) = INTRABAR_BASES
            .iter()
            .copied()
            .find(|(base, _)| *base <= interval && interval % base == 0)?;
        let (plain, modifier) = ticker::split(&key.symbol);
        let (source, symbol) = resolve_symbol(plain)?;
        let base_ms = base_seconds * 1_000;
        let mut start = chart
            .start_ms
            .max(chart.end_ms - base_ms * MAX_INTRABAR_CANDLES as i64);
        if key.recent_bars > 0 {
            start = start.max(chart.end_ms - chart.spacing_ms * key.recent_bars as i64);
        }
        let candles = crate::service::fetch_klines_range(
            source,
            &symbol,
            token,
            start,
            chart.end_ms,
            MAX_INTRABAR_CANDLES,
        )
        .ok()
        .filter(|candles| !candles.is_empty())?;
        let candles = if base_seconds == interval {
            candles
        } else {
            fold_candles(&candles, interval)
        };
        Some(construct(modifier, candles))
    }

    fn series(key: &SecurityKey, chart: Option<ChartSpan>) -> Option<Vec<Kline>> {
        // A `ticker.renko()` and its kin carry the construction with the symbol: the real candles
        // are fetched and the chart's own builders make the bars.
        let (plain, modifier) = ticker::split(&key.symbol);
        let (source, symbol) = resolve_symbol(plain)?;
        let interval = resolve_interval(key.interval)?;
        // Enough candles for the chart's span plus the expression's own lookback, up to the
        // service's page.
        let limit = chart.map_or(500, |chart| {
            let covering = (chart.end_ms - chart.start_ms) / (key.interval as i64 * 1_000).max(1);
            (covering + 300).clamp(500, 1_000) as u32
        });
        let candles = crate::service::fetch_klines(source, &symbol, interval, limit).ok()?;
        Some(construct(modifier, candles))
    }
}

impl SecurityDataProvider for KlineSecurityDataProvider {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        let cache_key = (key.clone(), None);
        if let Some(candles) = Self::cached(&cache_key) {
            return Some(candles);
        }
        let candles = Self::series(key, None)?;
        Self::remember(cache_key, &candles);
        Some(candles)
    }

    fn candles_over(&self, key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
        let cache_key = (key.clone(), Some(chart));
        if let Some(candles) = Self::cached(&cache_key) {
            return Some(candles);
        }
        let finer = (key.interval * 1_000.0) < chart.spacing_ms as f64;
        let candles = if finer {
            Self::intrabars(key, chart).or_else(|| Self::series(key, Some(chart)))?
        } else {
            Self::series(key, Some(chart))?
        };
        Self::remember(cache_key, &candles);
        Some(candles)
    }
}

/// The bars `modifier` builds from the real `candles`, or the candles themselves for a plain ticker.
fn construct(modifier: Option<ticker::TickerModifier>, candles: Vec<Kline>) -> Vec<Kline> {
    match modifier {
        Some(modifier) => ticker::apply(&modifier, &candles, now_millis()),
        None => candles,
    }
}

/// `candles` folded into candles of `interval` seconds: one per calendar bucket, the open of the
/// first, the extremes, the close of the last and the summed volume.
fn fold_candles(candles: &[Kline], interval: i64) -> Vec<Kline> {
    let mut folded: Vec<Kline> = Vec::new();
    for candle in candles {
        let bucket = crate::service::bucket_start_ms(candle.open_time, interval);
        match folded.last_mut() {
            Some(last) if last.open_time == bucket => {
                last.high = last.high.max(candle.high);
                last.low = last.low.min(candle.low);
                last.close = candle.close;
                last.volume += candle.volume;
            }
            _ => folded.push(Kline {
                open_time: bucket,
                close_time: bucket + interval * 1_000 - 1,
                open: candle.open,
                high: candle.high,
                low: candle.low,
                close: candle.close,
                volume: candle.volume,
            }),
        }
    }
    folded
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

/// `SOURCE:SYMBOL` resolves to that source; a bare symbol defaults to Binance, the chart's default
/// source.
fn resolve_symbol(symbol: &str) -> Option<(DataSource, String)> {
    if let Some((prefix, rest)) = symbol.split_once(':') {
        return Some((source_for_slug(prefix)?, rest.to_owned()));
    }
    Some((DataSource::Binance, symbol.to_owned()))
}

fn source_for_slug(slug: &str) -> Option<DataSource> {
    match slug.to_ascii_uppercase().as_str() {
        "BINANCE" => Some(DataSource::Binance),
        "COINBASE" => Some(DataSource::Coinbase),
        "COINGECKO" => Some(DataSource::CoinGecko),
        "DEXSCREENER" => Some(DataSource::DexScreener),
        "ALPACA" => Some(DataSource::Alpaca),
        "POLYMARKET" => Some(DataSource::Polymarket),
        "KALSHI" => Some(DataSource::Kalshi),
        "CMC" => Some(DataSource::CoinMarketCap),
        _ => None,
    }
}

/// The kline-service interval token for a bar length in seconds. None for a length the service does
/// not serve directly.
fn resolve_interval(seconds: f64) -> Option<&'static str> {
    const TOKENS: &[(f64, &str)] = &[
        (60.0, "1m"),
        (300.0, "5m"),
        (900.0, "15m"),
        (1_800.0, "30m"),
        (3_600.0, "1h"),
        (14_400.0, "4h"),
        (86_400.0, "1d"),
        (604_800.0, "1w"),
        (2_592_000.0, "1M"),
        (7_776_000.0, "3M"),
        (31_536_000.0, "1Y"),
    ];
    TOKENS
        .iter()
        .find(|(length, _)| (*length - seconds).abs() < 0.5)
        .map(|(_, token)| *token)
}

/// Compiles `source` and runs it once over every bar (historical phase), returning the chart
/// output. A script with syntax/type errors yields an empty output plus the program's diagnostics.
pub fn run_script(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> VisualOutput {
    run_script_with_inputs(source, bars, symbol, &HashMap::new())
}

/// `run_script` with the user's input overrides (by input id); anything absent runs at its
/// default. `import`s resolve against the Script Manager's library scripts.
pub fn run_script_with_inputs(
    source: &str,
    bars: &[Kline],
    symbol: &SymbolInfo,
    inputs: &HashMap<String, InputValue>,
) -> VisualOutput {
    run_script_checked(source, bars, symbol, inputs).0
}

/// `run_script_with_inputs`, also returning the runtime failure that stopped the run early, if
/// any. The output is whatever the script had drawn up to that bar.
pub fn run_script_checked(
    source: &str,
    bars: &[Kline],
    symbol: &SymbolInfo,
    inputs: &HashMap<String, InputValue>,
) -> (VisualOutput, Option<Diagnostic>) {
    run_script_checked_in_range(source, bars, symbol, inputs, None)
}

/// `run_script_checked` with the chart's first and last visible bar opening times. Scripts that
/// read `chart.left_visible_bar_time` or `chart.right_visible_bar_time` are rerun when this range
/// changes by the chart host.
pub fn run_script_checked_in_range(
    source: &str,
    bars: &[Kline],
    symbol: &SymbolInfo,
    inputs: &HashMap<String, InputValue>,
    visible_range: Option<(i64, i64)>,
) -> (VisualOutput, Option<Diagnostic>) {
    let program = compiler::compile(source, Limits::default(), &LibraryRegistry);
    if !program.is_valid() || program.statements.is_empty() {
        let failure = program
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .cloned();
        return (
            VisualOutput {
                overlay: false,
                bar_count: 0,
                ..Default::default()
            },
            failure,
        );
    }
    let mut session = Session::new(
        program,
        inputs.clone(),
        Limits::default(),
        None,
        symbol.clone(),
        Some(Box::new(KlineSecurityDataProvider)),
        visible_range,
    );
    // History loads the way the Swift controller does: the bar length and last bar index are
    // known from the first bar on, so `timeframe.*` and `barstate.islast` read correctly.
    let failure = match session.load_history(bars, false, None) {
        Err(RunError::Script(diagnostic)) => Some(diagnostic),
        Ok(_) | Err(RunError::Rejection(_)) => None,
    };
    (session.output(), failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Vec<Kline> {
        (0..5)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 102.0 + i as f64,
                volume: 1_000.0,
            })
            .collect()
    }

    fn minute(index: i64, open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: index * 60_000,
            close_time: index * 60_000 + 59_999,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn minute_candles_fold_into_calendar_buckets() {
        // Six 1-minute candles starting on a 5-minute boundary: one full 3-minute pair of buckets.
        let candles = [
            minute(0, 10.0, 11.0, 9.0, 10.5),
            minute(1, 10.5, 12.0, 10.0, 11.0),
            minute(2, 11.0, 11.5, 8.0, 9.0),
            minute(3, 9.0, 9.5, 8.5, 9.2),
            minute(4, 9.2, 10.0, 9.0, 9.9),
        ];
        let folded = fold_candles(&candles, 180);
        assert_eq!(folded.len(), 2);
        assert_eq!(
            (folded[0].open_time, folded[0].close_time),
            (0, 179_999),
            "the bucket spans its three minutes"
        );
        assert_eq!(
            (
                folded[0].open,
                folded[0].high,
                folded[0].low,
                folded[0].close
            ),
            (10.0, 12.0, 8.0, 9.0)
        );
        assert_eq!(folded[0].volume, 3.0);
        assert_eq!(
            (
                folded[1].open_time,
                folded[1].open,
                folded[1].close,
                folded[1].volume
            ),
            (180_000, 9.0, 9.9, 2.0),
            "a partial last bucket still folds"
        );
    }

    #[test]
    fn a_plot_script_produces_output() {
        let source = "//@version=6\nindicator(\"T\")\nplot(close)";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.bar_count >= 5);
        assert_eq!(output.plots.len(), 1);
    }

    #[test]
    fn compilation_failure_is_reported_as_a_failed_run() {
        let (output, failure) = run_script_checked(
            "//@version=6\nindicator(\"Broken\")\nplot(close",
            &bars(),
            &SymbolInfo::default(),
            &HashMap::new(),
        );

        assert_eq!(output.bar_count, 0);
        let failure = failure.expect("invalid source must not look like a successful empty run");
        assert_eq!(failure.severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn checked_run_exposes_the_chart_visible_range() {
        let (output, failure) = run_script_checked_in_range(
            "//@version=6\nindicator(\"Visible\")\nplot(chart.left_visible_bar_time)\nplot(chart.right_visible_bar_time)",
            &bars(),
            &SymbolInfo::default(),
            &HashMap::new(),
            Some((3_600_000, 10_800_000)),
        );

        assert!(failure.is_none());
        assert_eq!(output.plots[0].values.last(), Some(&Some(3_600_000.0)));
        assert_eq!(output.plots[1].values.last(), Some(&Some(10_800_000.0)));
    }

    #[test]
    fn the_bar_length_is_known_from_the_first_bar() {
        // On bar 0 `timeframe.period` used to read "0S", which `request.security` rejected.
        let source = "//@version=6\nindicator(\"T\")\n\
            higher = input.string(\"D\", \"Higher\", options = [\"60\", \"D\"])\n\
            usable = timeframe.in_seconds(higher) >= timeframe.in_seconds()\n\
            tf = usable ? higher : timeframe.period\n\
            plot(request.security(syminfo.tickerid, tf, ta.ema(close, 3)))";
        let (output, failure) =
            run_script_checked(source, &bars(), &SymbolInfo::default(), &HashMap::new());
        assert!(failure.is_none(), "{:?}", failure.map(|d| d.message));
        assert_eq!(output.plots.len(), 1);
    }

    #[test]
    fn a_strategy_that_reads_its_position_places_and_closes_trades() {
        // `strategy.exit` is placed only while `strategy.position_size > 0`, which read as na.
        let source = "//@version=6\nstrategy(\"T\")\n\
            if bar_index % 20 == 0\n    strategy.entry(\"L\", strategy.long)\n\
            if strategy.position_size > 0\n    \
            strategy.exit(\"X\", \"L\", stop = strategy.position_avg_price - 1, \
            limit = strategy.position_avg_price + 0.5)";
        let many: Vec<Kline> = (0..100)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1.0,
            })
            .collect();
        let (output, failure) =
            run_script_checked(source, &many, &SymbolInfo::default(), &HashMap::new());
        assert!(failure.is_none());
        let report = output.strategy.expect("a strategy report");
        assert!(report.trades.len() >= 4, "{} trades", report.trades.len());
    }

    #[test]
    fn an_invalid_script_produces_empty_output() {
        let source = "this is not pine";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.plots.is_empty());
    }

    #[test]
    fn mintick_is_inferred_from_low_priced_bars() {
        // Four-decimal prices infer a 0.0001 tick, not the 0.01 default.
        let low_bars: Vec<Kline> = (0..3)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 0.0001 + i as f64 * 0.0001,
                high: 0.0002 + i as f64 * 0.0001,
                low: 0.0001,
                close: 0.0001 + i as f64 * 0.0001,
                volume: 1_000.0,
            })
            .collect();
        let source = "//@version=6\nindicator(\"T\")\nplot(syminfo.mintick)";
        let output = run_script(source, &low_bars, &SymbolInfo::default());
        let last = output.plots[0].values.last().copied().flatten();
        assert_eq!(last, Some(0.0001));
    }

    #[test]
    fn security_symbols_resolve_to_a_source() {
        assert_eq!(
            resolve_symbol("ETHUSDT"),
            Some((DataSource::Binance, "ETHUSDT".to_owned()))
        );
        assert_eq!(
            resolve_symbol("BINANCE:ETHUSDT"),
            Some((DataSource::Binance, "ETHUSDT".to_owned()))
        );
        assert_eq!(
            resolve_symbol("COINBASE:BTC-USD"),
            Some((DataSource::Coinbase, "BTC-USD".to_owned()))
        );
        assert_eq!(resolve_symbol("NOPE:ETHUSDT"), None);
    }

    #[test]
    fn security_intervals_map_to_kline_tokens() {
        assert_eq!(resolve_interval(60.0), Some("1m"));
        assert_eq!(resolve_interval(3_600.0), Some("1h"));
        assert_eq!(resolve_interval(86_400.0), Some("1d"));
        assert_eq!(resolve_interval(2_592_000.0), Some("1M"));
        assert_eq!(resolve_interval(120.0), None); // 2m is not served directly
    }
}

#[cfg(test)]
mod call_site_tests {
    use super::*;

    fn bars(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|i| {
                let base = 100.0 + (i as f64 * 0.7).sin() * 10.0;
                Kline {
                    open_time: 1_700_000_000_000 + i as i64 * 86_400_000,
                    close_time: 1_700_000_000_000 + (i as i64 + 1) * 86_400_000 - 1,
                    open: base,
                    high: base + 2.0,
                    low: base - 2.0,
                    close: base + 0.5,
                    volume: 1000.0,
                }
            })
            .collect()
    }

    fn plots(source: &str) -> Vec<Vec<Option<f64>>> {
        let (output, failure) =
            run_script_checked(source, &bars(40), &SymbolInfo::default(), &HashMap::new());
        assert!(failure.is_none(), "{failure:?}");
        output.plots.into_iter().map(|p| p.values).collect()
    }

    #[test]
    fn two_ta_calls_in_one_script_keep_separate_history() {
        let found = plots(
            "//@version=6\nindicator(\"T\", overlay=true)\na = ta.sma(close, 3)\nb = ta.sma(close * 10, 3)\nplot(a)\nplot(b)\n",
        );
        let (a, b) = (&found[0], &found[1]);
        for index in 5..40 {
            let (x, y) = (a[index].unwrap(), b[index].unwrap());
            assert!((y - 10.0 * x).abs() < 1e-6, "bar {index}: {x} vs {y}");
        }
    }

    fn last_plot(source: &str, inputs: &HashMap<String, InputValue>) -> Option<f64> {
        let (output, failure) =
            run_script_checked(source, &bars(40), &SymbolInfo::default(), inputs);
        assert!(failure.is_none(), "{failure:?}");
        output.plots[0].values.last().copied().flatten()
    }

    #[test]
    fn input_calls_read_their_default_or_the_users_override() {
        let source = "//@version=6\nindicator(\"T\")\nlen = input.int(3, \"Length\")\nplot(ta.sma(close, len))\n";
        let default = last_plot(source, &HashMap::new()).expect("the default length computes");
        let mut overrides = HashMap::new();
        overrides.insert("len".to_owned(), InputValue::Int(10));
        let changed = last_plot(source, &overrides).expect("the override computes");
        assert!((default - changed).abs() > 1e-6, "{default} vs {changed}");
    }

    #[test]
    fn colour_and_source_inputs_resolve() {
        let colour = "//@version=6\nindicator(\"T\")\nc = input.color(color.teal, \"c\")\nplot(close, color = c)\n";
        let (output, _) =
            run_script_checked(colour, &bars(5), &SymbolInfo::default(), &HashMap::new());
        assert_eq!(
            output.plots[0].colors.last().copied().flatten(),
            Some(0x0089_7BFF)
        );
        let source = "//@version=6\nindicator(\"T\")\ns = input.source(close, \"s\")\nplot(s)\n";
        let mut overrides = HashMap::new();
        overrides.insert("s".to_owned(), InputValue::Source("open".into()));
        let open = last_plot(source, &overrides).unwrap();
        let close = last_plot(source, &HashMap::new()).unwrap();
        assert!(
            (close - open - 0.5).abs() < 1e-9,
            "close is open + 0.5 in the fixture"
        );
    }

    #[test]
    fn methods_on_a_collection_parameter_resolve() {
        let source = "//@version=6\nindicator(\"T\")\nadd(array<float> items, float v) =>\n    items.push(v)\n    items.size()\nvar array<float> xs = array.new<float>()\nplot(add(xs, close))\n";
        assert_eq!(last_plot(source, &HashMap::new()), Some(40.0));
    }

    #[test]
    fn cum_and_correlation_run_in_a_script() {
        let cum = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.cum(1))\n",
            &HashMap::new(),
        );
        assert_eq!(cum, Some(40.0), "one per bar over forty bars");
        let same = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.correlation(close, close * 2, 10))\n",
            &HashMap::new(),
        )
        .unwrap();
        assert!((same - 1.0).abs() < 1e-9, "{same}");
    }

    #[test]
    fn colour_components_gradients_fixnan_and_timeframe_strings_run() {
        let r = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(color.g(color.teal))\n",
            &HashMap::new(),
        );
        assert_eq!(r, Some(137.0));
        let t = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(color.t(color.new(color.red, 40)))\n",
            &HashMap::new(),
        )
        .unwrap();
        assert!((t - 40.0).abs() < 0.5, "{t}");
        let fixed = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(fixnan(bar_index % 5 == 0 ? close : na))\n",
            &HashMap::new(),
        );
        assert!(fixed.is_some(), "the gaps are filled");
        let tf = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(str.length(timeframe.from_seconds(3600)))\n",
            &HashMap::new(),
        );
        assert_eq!(tf, Some(2.0), "\"60\"");
        let tr = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.tr(true))\n",
            &HashMap::new(),
        );
        assert!(tr.is_some());
        let blended = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(color.r(color.from_gradient(5, 0, 10, color.black, color.white)))\n",
            &HashMap::new(),
        );
        assert_eq!(blended, Some(128.0));
    }

    #[test]
    fn barssince_vwap_dmi_and_runtime_error_run_in_a_script() {
        let since = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.barssince(bar_index == 30))\n",
            &HashMap::new(),
        );
        let index = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(bar_index)\n",
            &HashMap::new(),
        );
        assert_eq!(index, Some(39.0), "the first of forty bars is bar 0");
        assert_eq!(since, Some(9.0), "bar 39 is nine bars after bar 30");
        let vwap = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.vwap(close))\n",
            &HashMap::new(),
        );
        assert!(
            vwap.is_some_and(|v| (90.0..=111.0).contains(&v)),
            "{vwap:?}"
        );
        let dmi = last_plot(
            "//@version=6\nindicator(\"T\")\n[p, m, a] = ta.dmi(5, 5)\nplot(a)\n",
            &HashMap::new(),
        );
        assert!(dmi.is_some_and(|v| (0.0..=100.0).contains(&v)), "{dmi:?}");
        let (_, failure) = run_script_checked(
            "//@version=6\nindicator(\"T\")\nif bar_index == 3\n    runtime.error(\"stop here\")\n",
            &bars(10),
            &SymbolInfo::default(),
            &HashMap::new(),
        );
        let failure = failure.expect("the script stops");
        assert_eq!(
            (failure.code.as_str(), failure.message.as_str()),
            ("PINE4030", "stop here")
        );
    }

    #[test]
    fn the_first_bar_is_bar_zero_and_isfirst() {
        let (output, failure) = run_script_checked(
            "//@version=6\nindicator(\"T\")\nplot(barstate.isfirst ? 1 : 0)\nplot(bar_index)\n",
            &bars(5),
            &SymbolInfo::default(),
            &HashMap::new(),
        );
        assert!(failure.is_none());
        assert_eq!(output.plots[0].values[0], Some(1.0));
        assert_eq!(output.plots[0].values[1], Some(0.0));
        assert_eq!(output.plots[1].values[0], Some(0.0));
        assert_eq!(output.bar_count, 5);
    }

    #[test]
    fn format_time_follows_the_pattern_and_zone() {
        let source = "//@version=6\nindicator(\"T\")\nplot(str.length(str.format_time(1700000000000, \"yyyy-MM-dd HH:mm\", \"UTC\")))\n";
        assert_eq!(last_plot(source, &HashMap::new()), Some(16.0));
        let hours = "//@version=6\nindicator(\"T\")\nplot(str.tonumber(str.format_time(1700000000000, \"HH\", \"UTC+3\")))\n";
        assert_eq!(
            last_plot(hours, &HashMap::new()),
            Some(1.0),
            "22:13 UTC is 01:13 at UTC+3"
        );
    }

    #[test]
    fn timeframe_in_seconds_and_change_run_in_a_script() {
        let secs = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(timeframe.in_seconds(\"60\"))\n",
            &HashMap::new(),
        );
        assert_eq!(secs, Some(3600.0));
        let bar = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(timeframe.in_seconds())\n",
            &HashMap::new(),
        );
        assert_eq!(bar, Some(86_400.0), "the fixture's bars are a day apart");
        let changes = last_plot(
            "//@version=6\nindicator(\"T\")\nplot(ta.cum(timeframe.change(\"1D\") ? 1 : 0))\n",
            &HashMap::new(),
        );
        assert_eq!(changes, Some(40.0), "every daily bar opens a new day");
    }
}
