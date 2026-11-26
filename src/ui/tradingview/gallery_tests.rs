//! Pictures of the TradingView import's states, for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib tradingview_gallery -- --ignored`.

use super::*;
use crate::model::DataSource;
use crate::model::tradingview::plan::{self, Market, Match, Taken, Unmatched};
use crate::model::tradingview::{fixtures, parse};
use crate::ui::render_tests::Screen;
use crate::ui::theme;

fn market(source: DataSource, symbol: &str) -> Market {
    Market {
        source,
        symbol: symbol.into(),
        name: symbol.into(),
        image_url: None,
    }
}

fn frame(name: &str, view: &View<'_>) {
    let _theme = theme::RestoreDefaultTheme;
    for (theme_name, app_theme) in [
        ("dark", theme::AppTheme::Dark),
        ("light", theme::AppTheme::Light),
    ] {
        let mut screen = Screen::new(SIZE);
        egui_extras::install_image_loaders(&screen.ctx);
        theme::apply(&screen.ctx, app_theme);
        let mut last = None;
        // The mapping rows are below the first screen: scroll to them.
        let scroll = (name == "matching").then(|| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -2_000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        let pointer = egui::Event::PointerMoved(egui::pos2(300.0, 400.0));
        for step in 0..12 {
            let events = match &scroll {
                Some(wheel) if step == 1 => vec![pointer.clone(), wheel.clone()],
                _ => Vec::new(),
            };
            last = Some(screen.run(events, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                body(ui, view, &mut Vec::new());
            }));
        }
        screen.save(
            &format!("tradingview-{name}-{theme_name}"),
            last.expect("a frame"),
        );
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn tradingview_gallery() {
    let library = parse::library(fixtures::documents());
    let selection = plan::Selection::all(&library);
    let mut mappings = plan::Mappings::default();
    let symbol = |text: &str| TvSymbol::read(text).unwrap();
    mappings.found(
        symbol("BINANCE:BTCUSDT"),
        Match::Exact(market(DataSource::Binance, "BTCUSDT")),
    );
    mappings.found(
        symbol("OKX:ETHUSDT"),
        Match::Unmatched(Unmatched::Unreachable),
    );
    mappings.found(
        symbol("BITSTAMP:ETHUSD"),
        Match::Substitute(market(DataSource::Coinbase, "ETH-USD")),
    );
    mappings.found(symbol("NOPE:XYZ"), Match::Unmatched(Unmatched::Venue));
    let built = plan::plan(
        &library,
        &selection,
        &mappings,
        &Default::default(),
        &Taken::default(),
        &Default::default(),
    );
    let summary = Summary {
        layouts: vec![(1, "Majors".into())],
        drawings: 31,
        watchlists: 1,
        scripts: 2,
        failures: vec!["Coins: you already have the most watchlists".into()],
    };
    let content = |step| {
        View::Content(Content {
            opening: 1,
            folder: std::path::Path::new("/Users/me/Library/Application Support/TradingView"),
            library: &library,
            selection: &selection,
            mappings: &mappings,
            placing: false,
            unplaced: false,
            step,
            plan: (step == Step::Preview).then_some(&built),
        })
    };
    frame("choose", &content(Step::Choose));
    // The mapping rows on their own, as they are further down the first step.
    let nothing = plan::Selection {
        watchlists: selection.watchlists.clone(),
        layouts: selection.layouts.clone(),
        ..Default::default()
    };
    frame(
        "matching",
        &View::Content(Content {
            opening: 1,
            folder: std::path::Path::new("/tmp"),
            library: &Library {
                scripts: Vec::new(),
                ..library.clone()
            },
            selection: &nothing,
            mappings: &mappings,
            placing: false,
            unplaced: false,
            step: Step::Choose,
            plan: None,
        }),
    );
    frame("preview", &content(Step::Preview));
    frame("reading", &View::Reading);
    frame(
        "failed",
        &View::Failed("The TradingView app\u{2019}s data wasn\u{2019}t found on this computer."),
    );
    frame("done", &View::Done(&summary));
}
