//! Minimal Pine runtime host: compiles a script and runs it over a series of bars, returning the
//! chart `VisualOutput` the renderer consumes. Input resolution and library imports are wired
//! incrementally; the session itself owns the per-bar state machine. `chart` keeps the scripts on a
//! chart running in the background, `batch` runs a list of scripts once each, and `subscription`
//! runs the alert subscriptions.

pub mod batch;
pub mod chart;
pub mod subscription;

use crate::lock::lock;
use crate::model::chart::synthetic::ticker;
use crate::model::market::bucket;
use crate::model::{DataSource, Kline, Timeframe};
use crate::pine::compiler;
use crate::pine::compiler::program::{InputValue, SymbolInfo, VisualOutput};
use crate::pine::runtime::session::{RunError, Session};
use crate::pine::runtime::support::{
    ChartSpan, MAX_INTRABAR_CANDLES, SecurityDataProvider, SecurityKey,
};
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity, Limits};
use crate::pine::time::bucket::timeframe_of;
use crate::pine_studio::library::registry::LibraryRegistry;
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

/// The candle sizes the intrabar series are built from: a series of `request.security_lower_tf`
/// is folded from the coarsest of these that nests in its length (12 minutes from 1-minute
/// candles, 4 hours from 1-hour ones).
const INTRABAR_BASES: [Timeframe; 6] = [
    Timeframe::DAY_1,
    Timeframe::HOUR_1,
    Timeframe::MINUTE_30,
    Timeframe::MINUTE_15,
    Timeframe::MINUTE_5,
    Timeframe::MINUTE_1,
];

impl KlineSecurityDataProvider {
    fn cached(key: &(SecurityKey, Option<ChartSpan>)) -> Option<Vec<Kline>> {
        let cache = lock(&SECURITY_CACHE);
        let (candles, fetched_at) = cache.get(key)?;
        (now_millis() - fetched_at < SECURITY_CACHE_TTL_MS).then(|| candles.clone())
    }

    fn remember(key: (SecurityKey, Option<ChartSpan>), candles: &[Kline]) {
        lock(&SECURITY_CACHE).insert(key, (candles.to_vec(), now_millis()));
    }

    /// The series of finer bars over the chart's whole span (or its `calc_bars_count` newest bars).
    fn intrabars(key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
        let target = timeframe_of(key.interval)?;
        let base = bucket::fold_base(target, &INTRABAR_BASES)?;
        let (plain, modifier) = ticker::split(&key.symbol);
        let (source, symbol) = resolve_symbol(plain)?;
        let base_ms = base.nominal_ms();
        let mut start = chart
            .start_ms
            .max(chart.end_ms - base_ms * MAX_INTRABAR_CANDLES as i64);
        if key.recent_bars > 0 {
            start = start.max(chart.end_ms - chart.spacing_ms * key.recent_bars as i64);
        }
        let candles = crate::service::fetch_klines_range(
            source,
            &symbol,
            base,
            start,
            chart.end_ms,
            MAX_INTRABAR_CANDLES,
        )
        .ok()
        .filter(|candles| !candles.is_empty())?;
        let candles = if base == target {
            candles
        } else {
            bucket::fold(&candles, target, false)
        };
        Some(construct(modifier, candles))
    }

    fn series(key: &SecurityKey, chart: Option<ChartSpan>) -> Option<Vec<Kline>> {
        // A `ticker.renko()` and its kin carry the construction with the symbol: the real candles
        // are fetched and the chart's own builders make the bars.
        let (plain, modifier) = ticker::split(&key.symbol);
        let (source, symbol) = resolve_symbol(plain)?;
        let timeframe = timeframe_of(key.interval)?;
        // Enough candles for the chart's span plus the expression's own lookback, up to the
        // service's page.
        let limit = chart.map_or(500, |chart| {
            let covering = (chart.end_ms - chart.start_ms) / (key.interval as i64 * 1_000).max(1);
            (covering + 300).clamp(500, 1_000) as u32
        });
        let candles = crate::service::fetch_klines(source, &symbol, timeframe, limit).ok()?;
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
        "OKX" => Some(DataSource::Okx),
        "KRAKEN" => Some(DataSource::Kraken),
        "BYBIT" => Some(DataSource::Bybit),
        "GATEIO" => Some(DataSource::Gate),
        "MEXC" => Some(DataSource::Mexc),
        "COINGECKO" => Some(DataSource::CoinGecko),
        "DEXSCREENER" => Some(DataSource::DexScreener),
        "ALPACA" => Some(DataSource::Alpaca),
        "POLYMARKET" => Some(DataSource::Polymarket),
        "KALSHI" => Some(DataSource::Kalshi),
        "CMC" => Some(DataSource::CoinMarketCap),
        _ => None,
    }
}

/// Compiles `source` and runs it once over every bar (historical phase) with the user's input
/// overrides (by input id; anything absent runs at its default). Returns the chart output and the
/// runtime failure that stopped the run early, if any: the output is whatever the script had drawn
/// up to that bar, and empty for a script with syntax/type errors. `import`s resolve against the
/// Pine Studio's library scripts.
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
        Ok(_) | Err(RunError::Rejection) => None,
    };
    (session.output(), failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run at the script's default inputs.
    fn run_script(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> VisualOutput {
        run_script_checked(source, bars, symbol, &HashMap::new()).0
    }

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
