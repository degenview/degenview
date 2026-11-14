use super::*;
use crate::format;
use crate::pine::syntax::token::DiagnosticSeverity;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourcePosition, SourceRange};
use crate::pine_studio::workspace::PreviewInput;
use crate::store::Store;
use std::sync::{Arc, Mutex};

fn state() -> PreviewState {
    let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    PreviewState::load(&shared)
}

fn input(source: &str, errors: bool) -> PreviewInput {
    PreviewInput {
        name: "Demo".into(),
        script_type: ScriptType::Indicator,
        source: source.into(),
        problems: Vec::new(),
        diagnostics: if errors {
            vec![Diagnostic {
                code: "PINE1".into(),
                severity: DiagnosticSeverity::Error,
                category: DiagnosticCategory::Syntax,
                message: "bad".into(),
                range: SourceRange::new(SourcePosition::new(1, 1, 0), SourcePosition::new(1, 2, 1)),
            }]
        } else {
            Vec::new()
        },
        compile_pending: false,
    }
}

#[test]
fn an_edit_waits_for_its_compile_before_it_applies() {
    let mut preview = state();
    preview.script_changed("s1");
    let good = input("//@version=6\nindicator(\"x\")\nplot(close)\n", false);
    preview.source_changed(&good);
    preview.maybe_apply(&good);

    let mut edited = input("//@version=6\nindicator(\"x\")\nplot(open)\n", false);
    edited.compile_pending = true;
    preview.source_changed(&edited);
    preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
    preview.maybe_apply(&edited);
    assert_eq!(
        preview.applied_source.as_deref(),
        Some(good.source.as_str()),
        "the diagnostics still describe the old text"
    );
    assert!(preview.changed_at.is_some(), "the apply is kept for later");

    edited.compile_pending = false;
    preview.maybe_apply(&edited);
    assert_eq!(
        preview.applied_source.as_deref(),
        Some(edited.source.as_str())
    );
}

#[test]
fn a_new_market_or_timeframe_starts_on_autoscale_and_keeps_the_saved_zoom() {
    let mut preview = state();
    preview.card.set_visible_candles(80);
    preview.card.set_y_zoom(2.0);
    let stock = PreviewMarket {
        ticker: "AAPL".into(),
        source: DataSource::Alpaca,
    };

    // A range placed by dragging the old market's chart is in the old market's prices.
    preview.card.place_prices_for_test(90_000.0, 100_000.0);
    preview.select_market(stock.clone());
    assert!(preview.card.prices_are_autoscaled());
    assert_eq!(preview.card.visible_candles(), 80);
    assert_eq!(preview.card.y_zoom(), 2.0);

    preview.card.place_prices_for_test(240.0, 260.0);
    preview.select_timeframe(Timeframe::Week);
    assert!(preview.card.prices_are_autoscaled());

    // Choosing what is already shown leaves a placed range alone.
    preview.card.place_prices_for_test(240.0, 260.0);
    preview.select_market(stock);
    preview.select_timeframe(Timeframe::Week);
    assert!(!preview.card.prices_are_autoscaled());
}

#[test]
fn defaults_and_support() {
    let preview = state();
    assert_eq!(preview.market, PreviewMarket::default_market());
    assert_eq!(preview.timeframe, Timeframe::Day);
    assert!(PreviewMarket::is_supported(DataSource::Alpaca));
    assert!(!PreviewMarket::is_supported(DataSource::Polymarket));
    assert!(!PreviewMarket::is_supported(DataSource::CoinMarketCap));
}

#[test]
fn the_first_source_applies_at_once_then_edits_wait_for_a_pause() {
    let mut preview = state();
    preview.script_changed("s1");
    let good = input("//@version=6\nindicator(\"x\")\nplot(close)\n", false);
    preview.source_changed(&good);
    preview.maybe_apply(&good);
    assert_eq!(
        preview.applied_source.as_deref(),
        Some(good.source.as_str())
    );

    let edited = input("//@version=6\nindicator(\"x\")\nplot(open)\n", false);
    preview.source_changed(&edited);
    preview.maybe_apply(&edited);
    assert_eq!(
        preview.applied_source.as_deref(),
        Some(good.source.as_str()),
        "a keystroke ago: still the old source"
    );
    preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
    preview.maybe_apply(&edited);
    assert_eq!(
        preview.applied_source.as_deref(),
        Some(edited.source.as_str())
    );
}

#[test]
fn the_status_bar_is_busy_exactly_while_it_reads_evaluating() {
    let mut preview = state();
    assert!(!preview.is_busy(), "nothing applied is not busy");
    preview.script_changed("s1");
    let good = input("plot(close)", false);
    preview.source_changed(&good);
    preview.maybe_apply(&good);
    preview.loading = true;
    preview.candles.clear();
    assert_eq!(preview.status_text(), "Evaluating…");
    assert!(preview.is_busy());
    preview.loading = false;
    assert!(!preview.is_busy());
    assert_ne!(preview.status_text(), "Evaluating…");
    // A failure takes the place of the indicator, as on the chart's chip.
    preview.loading = true;
    preview.compile_failed = true;
    assert!(!preview.is_busy());
}

#[test]
fn broken_source_keeps_the_last_output_and_goes_stale() {
    let mut preview = state();
    preview.script_changed("s1");
    let good = input("plot(close)", false);
    preview.source_changed(&good);
    preview.maybe_apply(&good);
    let broken = input("plot(", true);
    preview.source_changed(&broken);
    preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
    preview.maybe_apply(&broken);
    assert!(preview.stale && preview.compile_failed);
    assert_eq!(preview.applied_source.as_deref(), Some("plot(close)"));
    assert_eq!(
        preview.status_text(),
        "Compile failed — last valid output remains active"
    );
    // Back to working text clears the banner.
    let fixed = input("plot(open)", false);
    preview.source_changed(&fixed);
    preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
    preview.maybe_apply(&fixed);
    assert!(!preview.stale && !preview.compile_failed);
}

#[test]
fn recents_are_newest_first_unique_and_capped() {
    let mut preview = state();
    for i in 0..7 {
        preview.select_market(PreviewMarket {
            ticker: format!("T{i}"),
            source: DataSource::Binance,
        });
    }
    assert_eq!(preview.recents.len(), RECENTS_LIMIT);
    assert_eq!(preview.recents[0].ticker, "T6");
    preview.select_market(PreviewMarket {
        ticker: "T4".into(),
        source: DataSource::Binance,
    });
    assert_eq!(preview.recents[0].ticker, "T4");
    assert_eq!(
        preview.recents.iter().filter(|m| m.ticker == "T4").count(),
        1
    );
}

#[test]
fn the_change_counts_closes_across_the_visible_window_only() {
    let mut preview = state();
    // 40 candles closing 100, 101, … 139; the first 25 are warm-up off the left edge.
    preview.candles = (0..40)
        .map(|i| Kline {
            open_time: i * 60_000,
            close_time: i * 60_000 + 59_999,
            open: 50.0,
            high: 200.0,
            low: 10.0,
            close: 100.0 + i as f64,
            volume: 1.0,
        })
        .collect();
    preview.card.set_visible_candles(15);
    let (price, change) = preview.price_line().unwrap();
    assert_eq!(price, format::price(139.0));
    // Close 125 → close 139, not the first candle's open of 50.
    assert!((change - (139.0 - 125.0) / 125.0 * 100.0).abs() < 1e-9);
}

#[test]
fn percent_uses_a_real_minus() {
    assert_eq!(signed_percent(1.5), "+1.50%");
    assert_eq!(signed_percent(-0.25), "−0.25%");
}
