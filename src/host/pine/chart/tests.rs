use super::*;

fn candle(open: f64, close: f64) -> Kline {
    Kline {
        open_time: 0,
        close_time: 59_999,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
        volume: 1.0,
    }
}

fn hourly(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + index as f64;
            Kline {
                open_time: index as i64 * 3_600_000,
                close_time: (index as i64 + 1) * 3_600_000 - 1,
                open: close - 1.0,
                high: close + 1.0,
                low: close - 2.0,
                close,
                volume: 10.0,
            }
        })
        .collect()
}

const PLAIN: &str = "//@version=6\nindicator(\"Plain\", overlay = true)\nplot(close)\n";

fn symbol() -> SymbolInfo {
    SymbolInfo {
        ticker: "TEST".into(),
        ticker_id: "BINANCE:TEST".into(),
        currency: "USD".into(),
        kind: "crypto".into(),
        ..SymbolInfo::default()
    }
}

fn frame<'a>(bars: &'a [Kline], symbol: &'a SymbolInfo) -> ChartFrame<'a> {
    ChartFrame {
        bars,
        symbol,
        timeframe: None,
        bar_seconds: None,
        theme: ChartTheme::DARK,
        visible_range: None,
    }
}

fn frame_in<'a>(bars: &'a [Kline], symbol: &'a SymbolInfo, timeframe: Timeframe) -> ChartFrame<'a> {
    ChartFrame {
        timeframe: Some(timeframe),
        ..frame(bars, symbol)
    }
}

/// Updates until the instance's output has drawn something, and returns it.
fn wait_for_plots(
    runs: &mut ChartRuns,
    chart: &ChartFrame<'_>,
    instances: &[PineInstance],
) -> Arc<VisualOutput> {
    let ctx = egui::Context::default();
    for _ in 0..500 {
        if let Some((output, ..)) = runs.update(&ctx, chart, instances).pop()
            && !output.plots.is_empty()
        {
            return output;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the script never drew");
}

/// Updates until the instance's run has a result.
fn wait(runs: &mut ChartRuns, chart: &ChartFrame<'_>, instances: &[PineInstance], id: u64) {
    let ctx = egui::Context::default();
    for _ in 0..500 {
        runs.update(&ctx, chart, instances);
        if runs.status(id).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("instance {id} did not finish");
}

#[test]
fn a_run_is_busy_only_once_it_is_slow() {
    let start = Instant::now();
    assert!(!is_slow(None, start + PINE_BUSY_AFTER * 10));
    assert!(!is_slow(Some(start), start));
    assert!(!is_slow(
        Some(start),
        start + PINE_BUSY_AFTER - Duration::from_millis(1)
    ));
    assert!(is_slow(Some(start), start + PINE_BUSY_AFTER));
}

#[test]
fn pine_revision_changes_for_intrabar_values_viewports_and_close_transition() {
    let first = candle(10.0, 11.0);
    let mut updated = first.clone();
    updated.close = 12.0;
    let base = RunRevision::of(std::slice::from_ref(&first), Some((0, 0)), 0);
    assert_ne!(base, RunRevision::of(&[updated], Some((0, 0)), 0));
    assert_ne!(base, RunRevision::of(std::slice::from_ref(&first), None, 0));
    assert_ne!(base, RunRevision::of(&[first], Some((0, 0)), 60_000));
}

#[test]
fn the_same_bars_are_what_equal_fingerprints_say() {
    let bars = hourly(50);
    assert!(same_bars(&bars, &bars.clone()));
    let mut inner = bars.clone();
    inner[10].volume = 11.0;
    assert!(!same_bars(&bars, &inner));
    assert_ne!(candles_fingerprint(&bars), candles_fingerprint(&inner));
    assert!(!same_bars(&bars, &bars[1..]));
    // Bits, not numbers: a negative zero is another bar, as it is another fingerprint.
    let mut zero = bars.clone();
    zero[0].volume = 0.0;
    let mut negative = bars.clone();
    negative[0].volume = -0.0;
    assert!(!same_bars(&zero, &negative));
    let mut nan = bars.clone();
    nan[0].close = f64::NAN;
    assert!(same_bars(&nan, &nan.clone()));
}

#[test]
fn the_bars_are_copied_and_hashed_again_only_when_they_change() {
    let mut runs = ChartRuns::default();
    let bars = hourly(20);
    let first = Arc::clone(&runs.handed_bars(&bars).bars);
    let again = Arc::clone(&runs.handed_bars(&bars.clone()).bars);
    assert!(
        Arc::ptr_eq(&first, &again),
        "the same bars are handed again"
    );
    let mut inner = bars.clone();
    inner[3].high += 1.0;
    let handed = runs.handed_bars(&inner);
    assert!(!Arc::ptr_eq(&first, &handed.bars));
    assert_eq!(handed.fingerprint, candles_fingerprint(&inner));
}

#[test]
fn a_run_restarts_when_any_bar_changes_and_never_for_the_same_bars() {
    let ctx = egui::Context::default();
    let symbol = symbol();
    let mut runs = ChartRuns::default();
    let instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    let bars = hourly(30);
    wait(&mut runs, &frame(&bars, &symbol), &instances, 1);
    let requested = runs.generation_for_tests(1).unwrap();
    runs.update(&ctx, &frame(&bars.clone(), &symbol), &instances);
    assert_eq!(runs.generation_for_tests(1), Some(requested));
    // A change behind the newest bar is a change of data too, as it was with every bar hashed.
    let mut inner = bars.clone();
    inner[5].close += 0.5;
    runs.update(&ctx, &frame(&inner, &symbol), &instances);
    assert_eq!(runs.generation_for_tests(1), Some(requested + 1));
}

#[test]
fn a_chart_with_no_visible_script_keeps_no_runs_and_no_bars() {
    let ctx = egui::Context::default();
    let symbol = symbol();
    let mut runs = ChartRuns::default();
    let mut instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    let bars = hourly(10);
    runs.update(&ctx, &frame(&bars, &symbol), &instances);
    assert!(runs.handed.is_some() && runs.generation_for_tests(1).is_some());
    instances[0].visible = false;
    assert!(
        runs.update(&ctx, &frame(&bars, &symbol), &instances)
            .is_empty()
    );
    assert!(runs.handed.is_none() && runs.generation_for_tests(1).is_none());
    assert!(!runs.busy(1, Instant::now()));
}

#[test]
fn a_restart_is_a_new_epoch_and_keeps_the_last_valid_output_up() {
    let symbol = symbol();
    let mut runs = ChartRuns::default();
    let bars = hourly(30);
    let mut instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    wait(&mut runs, &frame(&bars, &symbol), &instances, 1);
    let epoch = runs.logs(1).unwrap().epoch;
    assert!(runs.has_valid_output(1));
    instances[0].source =
        "//@version=6\nindicator(\"Broken\", overlay = true)\nplot(close / na_function())\n".into();
    let ctx = egui::Context::default();
    let shown = runs.update(&ctx, &frame(&bars, &symbol), &instances);
    assert_eq!(
        shown.len(),
        1,
        "the old output stays while the new run goes"
    );
    assert!(runs.logs(1).unwrap().epoch > epoch);
}

#[test]
fn output_of_another_timeframe_is_never_shown_over_the_new_bars() {
    let ctx = egui::Context::default();
    let symbol = symbol();
    let mut runs = ChartRuns::default();
    let instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    let hours = hourly(30);
    let drawn = wait_for_plots(
        &mut runs,
        &frame_in(&hours, &symbol, Timeframe::HOUR_1),
        &instances,
    );
    assert_eq!(drawn.bar_count, 30);
    assert!(runs.has_valid_output(1));

    // The next frame is the new timeframe's first: what the hourly run drew is not of these bars.
    let days = hourly(8);
    let in_days = frame_in(&days, &symbol, Timeframe::DAY_1);
    let shown = runs.update(&ctx, &in_days, &instances);
    assert_eq!(shown.len(), 1, "the script keeps its place on the chart");
    assert!(shown[0].0.plots.is_empty() && shown[0].0.bar_count == 0);
    assert!(shown[0].0.overlay, "and where it draws");
    assert!(!runs.has_valid_output(1));

    let redrawn = wait_for_plots(&mut runs, &in_days, &instances);
    assert_eq!(redrawn.bar_count, 8);
}

#[test]
fn output_of_another_market_or_chart_type_is_never_shown_either() {
    let ctx = egui::Context::default();
    let bars = hourly(30);
    let mut runs = ChartRuns::default();
    let instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    let first = symbol();
    wait_for_plots(
        &mut runs,
        &frame_in(&bars, &first, Timeframe::HOUR_1),
        &instances,
    );
    let other_market = SymbolInfo {
        ticker_id: "OKX:TEST".into(),
        ..symbol()
    };
    let shown = runs.update(
        &ctx,
        &frame_in(&bars, &other_market, Timeframe::HOUR_1),
        &instances,
    );
    assert!(shown[0].0.plots.is_empty());

    wait_for_plots(
        &mut runs,
        &frame_in(&bars, &other_market, Timeframe::HOUR_1),
        &instances,
    );
    let heikin_ashi = SymbolInfo {
        chart: crate::model::ChartKind::HeikinAshi,
        ..other_market
    };
    let shown = runs.update(
        &ctx,
        &frame_in(&bars, &heikin_ashi, Timeframe::HOUR_1),
        &instances,
    );
    assert!(shown[0].0.plots.is_empty());
}

#[test]
fn the_same_series_keeps_its_output_through_new_bars() {
    let ctx = egui::Context::default();
    let symbol = symbol();
    let mut runs = ChartRuns::default();
    let instances = vec![PineInstance::new(1, None, "Plain", PLAIN)];
    let bars = hourly(30);
    wait_for_plots(
        &mut runs,
        &frame_in(&bars, &symbol, Timeframe::HOUR_1),
        &instances,
    );
    // A live tick or an older page changes the bars, not what they are: no flash.
    let more = hourly(31);
    let shown = runs.update(
        &ctx,
        &frame_in(&more, &symbol, Timeframe::HOUR_1),
        &instances,
    );
    assert_eq!(shown[0].0.bar_count, 30);
    assert!(runs.has_valid_output(1));
}
