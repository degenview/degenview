//! A script sees the chart it runs on: the real candles on every standard chart type, the
//! constructed bars on a price-based one. `chart.is_*`, series history, `request.security()` and
//! the `ticker.*` builders are checked end to end through the real compiler and session.

use crate::model::chart::kind::{ConstructionSource, SizeMethod, SizeRule};
use crate::model::chart::synthetic::ticker::{self, TickerModifier};
use crate::model::chart::synthetic::{Construction, build_with};
use crate::model::{ChartKind, Kline};
use crate::pine::compiler::program::{BarEvent, BarPhase, SymbolInfo, VisualOutput};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::runtime::session::Session;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};
use crate::pine::syntax::token::Limits;
use std::collections::HashMap;

const HOUR: i64 = 3_600_000;

fn candle(index: usize, close: f64) -> Kline {
    Kline {
        open_time: index as i64 * HOUR,
        close_time: (index as i64 + 1) * HOUR - 1,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
    }
}

fn candles(closes: &[f64]) -> Vec<Kline> {
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| candle(index, *close))
        .collect()
}

fn traditional(amount: f64) -> SizeRule {
    SizeRule {
        method: SizeMethod::Traditional,
        amount,
        ..SizeRule::default()
    }
}

fn renko(size: f64) -> Construction {
    Construction::Renko {
        size: traditional(size),
        source: ConstructionSource::Close,
        wicks: false,
    }
}

struct Provider(HashMap<String, Vec<Kline>>);

impl SecurityDataProvider for Provider {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        self.0.get(&key.symbol).cloned()
    }
}

/// The provider a chart has: a symbol's real candles, and any `ticker.*` of it built the way the
/// app's provider builds it.
struct Market(Vec<Kline>);

impl SecurityDataProvider for Market {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        let (_, modifier) = ticker::split(&key.symbol);
        Some(match modifier {
            Some(modifier) => ticker::apply(&modifier, &self.0, i64::MAX / 2),
            None => self.0.clone(),
        })
    }
}

fn run(
    source: &str,
    bars: &[Kline],
    chart: ChartKind,
    provider: Option<Box<dyn SecurityDataProvider>>,
) -> VisualOutput {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let symbol = SymbolInfo {
        ticker: "BTCUSDT".to_owned(),
        ticker_id: "BINANCE:BTCUSDT".to_owned(),
        chart,
        ..SymbolInfo::default()
    };
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol,
        provider,
        None,
    );
    session.fixed_bar_seconds = Some(3_600.0);
    session
        .load_history(bars, false, None)
        .expect("history runs");
    session.output()
}

fn last(output: &VisualOutput, plot: usize) -> Option<f64> {
    output.plots[plot].values.last().copied().flatten()
}

#[test]
fn chart_is_reports_the_kind_and_only_that_kind() {
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(chart.is_standard ? 1 : 0)\nplot(chart.is_heikinashi ? 1 : 0)\n\
        plot(chart.is_renko ? 1 : 0)\nplot(chart.is_kagi ? 1 : 0)\n\
        plot(chart.is_linebreak ? 1 : 0)\nplot(chart.is_pnf ? 1 : 0)\nplot(chart.is_range ? 1 : 0)";
    let bars = candles(&[1.0, 2.0]);
    let kinds = [
        ChartKind::Standard,
        ChartKind::HeikinAshi,
        ChartKind::Renko,
        ChartKind::Kagi,
        ChartKind::LineBreak,
        ChartKind::PointAndFigure,
        ChartKind::Range,
    ];
    for (position, kind) in kinds.into_iter().enumerate() {
        let output = run(source, &bars, kind, None);
        for plot in 0..kinds.len() {
            let expected = if plot == position { 1.0 } else { 0.0 };
            assert_eq!(last(&output, plot), Some(expected), "{kind:?} plot {plot}");
        }
    }
}

#[test]
fn history_and_indexes_run_over_the_constructed_bars() {
    // Real closes 10, 12.5, 11, 9 make four bricks of 1: 11, 12, 10, 9.
    let real = candles(&[10.0, 12.5, 11.0, 9.0]);
    let bricks = build_with(renko(1.0), &real, i64::MAX / 2).klines();
    assert_eq!(bricks.len(), 4);
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(bar_index)\nplot(close[1])\nplot(ta.sma(close, 2))";
    let on_renko = run(source, &bricks, ChartKind::Renko, None);
    assert_eq!(last(&on_renko, 0), Some(3.0));
    assert_eq!(last(&on_renko, 1), Some(10.0), "the brick before the last");
    assert_eq!(last(&on_renko, 2), Some(9.5), "(10 + 9) / 2 over bricks");
    // The same script on the real candles: three candles back is nothing like the bricks.
    let on_candles = run(source, &real, ChartKind::Standard, None);
    assert_eq!(last(&on_candles, 1), Some(11.0));
    assert_eq!(last(&on_candles, 2), Some(10.0));
}

#[test]
fn a_standard_ticker_on_a_price_based_chart_reads_the_market_not_the_chart() {
    let real = candles(&[10.0, 12.5, 11.0, 9.0]);
    let bricks = build_with(renko(1.0), &real, i64::MAX / 2).klines();
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(close)\nplot(request.security(syminfo.tickerid, timeframe.period, close))";
    let provider = Provider(HashMap::from([(
        "BINANCE:BTCUSDT".to_owned(),
        real.clone(),
    )]));
    let output = run(source, &bricks, ChartKind::Renko, Some(Box::new(provider)));
    assert_eq!(last(&output, 0), Some(9.0), "the brick's close");
    assert_eq!(last(&output, 1), Some(9.0), "the market's close");
    // Where they differ: ask at the second bar, whose brick closes at 12 and candle at 12.5.
    let first_two = &bricks[..2];
    let provider = Provider(HashMap::from([("BINANCE:BTCUSDT".to_owned(), real)]));
    let output = run(
        source,
        first_two,
        ChartKind::Renko,
        Some(Box::new(provider)),
    );
    assert_eq!(last(&output, 0), Some(12.0));
    assert_eq!(
        last(&output, 1),
        Some(12.5),
        "the market's candle, not the brick"
    );
}

#[test]
fn ticker_builders_return_modified_tickers_the_security_feed_serves() {
    let real = candles(&[10.0, 12.5, 11.0, 9.0]);
    let source = "//@version=6\nindicator(\"T\")\n\
        ha = ticker.heikinashi(syminfo.tickerid)\n\
        rk = ticker.renko(syminfo.tickerid, \"Traditional\", 1)\n\
        plot(request.security(ha, timeframe.period, close))\n\
        plot(request.security(rk, timeframe.period, close))\n\
        plot(request.security(syminfo.tickerid, timeframe.period, close))";
    let output = run(
        source,
        &real,
        ChartKind::Standard,
        Some(Box::new(Market(real.clone()))),
    );
    // Heikin Ashi close of a flat candle is its own value; the last candle is 9.
    assert_eq!(last(&output, 0), Some(9.0));
    // The last brick of a 1-box Renko over these closes closes at 9.
    assert_eq!(last(&output, 1), Some(9.0));
    assert_eq!(last(&output, 2), Some(9.0));
    // Bricks are not candles: the series differ in length, which the plain ticker does not show.
    let bricks = ticker::apply(
        &TickerModifier::Renko {
            size: traditional(1.0),
            wicks: false,
            source: ConstructionSource::Close,
        },
        &real,
        i64::MAX / 2,
    );
    assert_eq!(bricks.len(), 4);
}

#[test]
fn a_ticker_builder_given_nonsense_returns_na_rather_than_the_plain_symbol() {
    let real = candles(&[10.0, 11.0]);
    let source = "//@version=6\nindicator(\"T\")\n\
        bad = ticker.renko(syminfo.tickerid, \"Nope\", 1)\n\
        plot(na(bad) ? 1 : 0)";
    let output = run(
        source,
        &real,
        ChartKind::Standard,
        Some(Box::new(Market(real.clone()))),
    );
    assert_eq!(last(&output, 0), Some(1.0));
}

#[test]
fn the_controller_keeps_the_source_bar_length_for_constructed_bars() {
    let real = candles(&[10.0, 13.5]);
    let bricks = build_with(renko(1.0), &real, i64::MAX / 2).klines();
    // Two bricks made by one candle are a millisecond apart: their times say nothing of the
    // timeframe.
    assert_eq!(bricks.len(), 3);
    assert_eq!(bricks[2].open_time - bricks[1].open_time, 1);
    let mut controller = ExecutionController::new(
        "//@version=6\nindicator(\"T\")\nplot(timeframe.in_seconds())",
        HashMap::new(),
        SymbolInfo {
            chart: ChartKind::Renko,
            ..SymbolInfo::default()
        },
        None,
        Limits::default(),
        &NoLibraries,
        None,
        None,
    )
    .expect("compiles")
    .with_bar_seconds(3_600.0);
    let update = controller.update(&bricks, None, i64::MAX / 2);
    assert_eq!(update.output.plots[0].values.last(), Some(&Some(3_600.0)));
}

#[test]
fn a_strategy_on_a_price_based_chart_says_its_fills_are_constructed_prices() {
    let source = "//@version=6\nstrategy(\"S\", overlay = true)\n\
        if bar_index == 1\n    strategy.entry(\"L\", strategy.long)\n\
        if bar_index == 3\n    strategy.close(\"L\")";
    let real = candles(&[10.0, 12.5, 11.0, 9.0, 8.0]);
    let bricks = build_with(renko(1.0), &real, i64::MAX / 2).klines();
    let on_renko = run(source, &bricks, ChartKind::Renko, None);
    let report = on_renko.strategy.expect("a strategy report");
    assert_eq!(report.chart, ChartKind::Renko);
    assert!(crate::ui::pine_report::synthetic_fill_notice(report.chart)
        .is_some_and(|notice| notice.contains("Renko") && notice.contains("constructed prices")));
    // On the market's own candles there is nothing to caveat.
    let on_candles = run(source, &real, ChartKind::Standard, None);
    assert_eq!(
        on_candles.strategy.expect("report").chart,
        ChartKind::Standard
    );
    assert!(crate::ui::pine_report::synthetic_fill_notice(ChartKind::Standard).is_none());
}
