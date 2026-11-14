//! The watchlist sidebar as pictures, in both themes: the default columns with the month's chart,
//! a wider list with every price column, the chart off, compact rows and loading charts. Run by
//! hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib watchlist::gallery -- --ignored`.

use crate::model::DataSource;
use crate::model::InstrumentId;
use crate::model::market::trend::Trend;
use crate::model::watchlist::layout::Flags;
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::model::watchlist::{Watchlist, WatchlistColumn, WatchlistInstrument};
use crate::ui::render_tests::Screen;
use crate::ui::theme;
use crate::ui::watchlist::sidebar::{self, WatchlistSidebarInput, WatchlistSidebarState};
use crate::ui::watchlist::style;
use eframe::egui::{self, Vec2};
use std::collections::HashMap;

/// Symbol, name as a search hit gives it, price, day's change, volume, and the month's drift.
const MARKETS: [(DataSource, &str, &str, f64, f64, f64, f32); 8] = [
    (
        DataSource::Binance,
        "BTCUSDT",
        "BTCUSDT",
        67_099.99,
        -1_214.01,
        9_012.0,
        -0.08,
    ),
    (
        DataSource::Binance,
        "ETHUSDT",
        "ETHUSDT",
        1_898.22,
        -31.66,
        119_900.0,
        -0.12,
    ),
    (
        DataSource::Binance,
        "SOLUSDT",
        "SOLUSDT",
        75.26,
        -2.93,
        1_600_000.0,
        -0.2,
    ),
    (
        DataSource::Binance,
        "DOGEUSDT",
        "DOGEUSDT",
        0.06261,
        -0.00193,
        230_110_000.0,
        -0.1,
    ),
    (
        DataSource::Alpaca,
        "NVDA — NVIDIA Corporation",
        "NVDA",
        161.03,
        -7.0,
        17_240_000.0,
        -0.06,
    ),
    (
        DataSource::Alpaca,
        "AAPL — Apple Inc.",
        "AAPL",
        355.33,
        0.06,
        1_310_000.0,
        0.05,
    ),
    (
        DataSource::Alpaca,
        "TSLA — Tesla, Inc.",
        "TSLA",
        387.16,
        -12.34,
        1_710_000.0,
        0.04,
    ),
    (
        DataSource::Binance,
        "BNBUSDT",
        "BNBUSDT",
        614.74,
        -15.57,
        63_800.0,
        -0.05,
    ),
];

const DEFAULT_COLUMNS: [WatchlistColumn; 2] =
    [WatchlistColumn::Last, WatchlistColumn::ChangePercent];
const PRICE_COLUMNS: [WatchlistColumn; 3] = [
    WatchlistColumn::Last,
    WatchlistColumn::ChangePercent,
    WatchlistColumn::Volume,
];

/// A month of closes that wanders around a drift, the same for a market every run.
fn month(seed: usize, drift: f32) -> Vec<f32> {
    let mut level = 100.0_f32;
    (0..30)
        .map(|day| {
            let wobble = (((day * 7 + seed * 13) % 11) as f32 - 5.0) * 0.6;
            level += drift * 100.0 / 30.0 + wobble;
            level
        })
        .collect()
}

fn quote(last: f64, change: f64, volume: f64) -> WatchlistQuote {
    WatchlistQuote {
        last: Some(last),
        change: Some(change),
        change_percent: Some(change / (last - change) * 100.0),
        volume: Some(volume),
        freshness: Freshness::Live,
        ..Default::default()
    }
}

struct Case {
    name: &'static str,
    columns: &'static [WatchlistColumn],
    shows_chart: bool,
    shows_description: bool,
    /// Extra width over the list's minimum.
    extra: f32,
    /// Trends known: false shows every chart still loading.
    loaded: bool,
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_sidebar() {
    let _restore = theme::RestoreDefaultTheme;
    let cases = [
        Case {
            name: "default",
            columns: &DEFAULT_COLUMNS,
            shows_chart: true,
            shows_description: true,
            extra: 0.0,
            loaded: true,
        },
        Case {
            name: "volume-wide",
            columns: &PRICE_COLUMNS,
            shows_chart: true,
            shows_description: true,
            extra: 60.0,
            loaded: true,
        },
        Case {
            name: "chart-off",
            columns: &DEFAULT_COLUMNS,
            shows_chart: false,
            shows_description: true,
            extra: 0.0,
            loaded: true,
        },
        Case {
            name: "compact",
            columns: &DEFAULT_COLUMNS,
            shows_chart: true,
            shows_description: false,
            extra: 0.0,
            loaded: true,
        },
        Case {
            name: "loading",
            columns: &DEFAULT_COLUMNS,
            shows_chart: true,
            shows_description: true,
            extra: 0.0,
            loaded: false,
        },
    ];

    let mut list = Watchlist::new("Favorites");
    let mut quotes = HashMap::new();
    let mut trends = HashMap::new();
    for (index, (source, symbol, id, last, change, volume, drift)) in MARKETS.iter().enumerate() {
        let item = WatchlistInstrument::from_market(*source, symbol, id);
        quotes.insert(item.instrument.key(), quote(*last, *change, *volume));
        trends.insert(item.instrument.key(), Trend::Ready(month(index, *drift)));
        list.add(item, None).expect("added");
    }
    let second = list.instruments().nth(1).map(|item| item.id);
    let lookup = |id: &InstrumentId| quotes.get(&id.key()).cloned();
    let flags = Flags::default();
    let no_trends = HashMap::new();

    for case in cases {
        let mut list = list.clone();
        list.display.columns = case.columns.to_vec();
        list.display.shows_chart = case.shows_chart;
        list.display.shows_description = case.shows_description;
        let width = style::minimum_panel_width(&list.display) + case.extra;
        let lists = vec![list];
        for light in [false, true] {
            let mut screen = Screen::new(Vec2::new(width, 520.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut state = WatchlistSidebarState::default();
            state.selected_entry = second;
            let input = WatchlistSidebarInput {
                lists: &lists,
                flags: &flags,
                quote: &lookup,
                trends: if case.loaded { &trends } else { &no_trends },
                focused_market: None,
                load_failed: false,
                error: None,
                import_report: None,
            };
            let mut last = None;
            // Hover the fifth row, so the pictures show a lit row beside the hairlines.
            let hover = egui::pos2(width * 0.5, 280.0);
            for _ in 0..4 {
                last = Some(screen.run(vec![egui::Event::PointerMoved(hover)], |ui| {
                    theme::sync(ui.ctx());
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(theme::chrome()))
                        .show(ui, |ui| {
                            sidebar::show(ui, &mut state, &input);
                        });
                }));
            }
            let theme_name = if light { "light" } else { "dark" };
            screen.save(
                &format!("watchlist-{}-{theme_name}", case.name),
                last.expect("a frame"),
            );
        }
    }
}
