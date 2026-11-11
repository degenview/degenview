//! Pictures of the four portfolio cards on the chart grid, for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib portfolio_cards_gallery -- --ignored`.
//! Each picture is a sheet of cards at several sizes drawn with the grid's own card frame, in
//! both themes, with and without a pointer over the card.

use super::{body, header};
use crate::model::DataSource;
use crate::model::portfolio::card::{AssetMove, CardData, CardKind, PortfolioCard, performance};
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::{
    PortfolioHistoryRange, PortfolioPeriodChange, PortfolioValueChange,
};
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioHolding, PortfolioSnapshot,
};
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::{ChartCardAction, theme};
use eframe::egui::{self, Pos2, Rect, Vec2, vec2};
use std::collections::HashMap;

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;

fn holding(ticker: &str, name: &str, value: f64, cost: f64, allocation: f64) -> PortfolioHolding {
    PortfolioHolding {
        asset: PortfolioAsset {
            key: format!("coinmarketcap:{ticker}"),
            symbol: ticker.to_owned(),
            name: name.to_owned(),
            source: DataSource::CoinMarketCap,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        },
        quantity: 1.0,
        cost_basis: cost,
        average_cost: cost,
        realized_pnl: 0.0,
        current_price: Some(value),
        previous_day_price: Some(value * 0.98),
        current_value: Some(value),
        unrealized_pnl: Some(value - cost),
        allocation,
        is_price_stale: false,
    }
}

fn holdings(count: usize) -> Vec<PortfolioHolding> {
    let all = [
        ("BTC", "Bitcoin", 8_120.5, 5_900.0, 0.52),
        ("ETH", "Ethereum", 3_410.0, 3_050.0, 0.22),
        ("SOL", "Solana", 1_850.0, 1_990.0, 0.12),
        ("LINK", "Chainlink", 1_240.0, 800.0, 0.08),
        ("ARB", "Arbitrum", 640.0, 790.0, 0.04),
        ("DOGE", "Dogecoin", 320.0, 400.0, 0.02),
        ("AVAX", "Avalanche", 210.0, 190.0, 0.01),
        ("ATOM", "Cosmos", 90.0, 120.0, 0.005),
        ("OP", "Optimism", 60.0, 50.0, 0.003),
        ("XRP", "XRP", 40.0, 35.0, 0.002),
    ];
    all.iter()
        .take(count)
        .map(|(t, n, v, c, a)| holding(t, n, *v, *c, *a))
        .collect()
}

fn history(range: PortfolioHistoryRange, up: bool) -> Vec<PortfolioSnapshot> {
    let (count, step) = match range {
        PortfolioHistoryRange::OneDay => (48, DAY / 48),
        _ => (120, DAY),
    };
    (0..count)
        .map(|i| {
            let wave = (i as f64 * 0.21).sin() * 520.0 + (i as f64 * 0.07).cos() * 380.0;
            let slope = if up { 28.0 } else { -14.0 };
            let value = 12_000.0 + i as f64 * slope + wave;
            PortfolioSnapshot {
                portfolio_id: 1,
                timestamp: NOW - (count as i64 - i as i64) * step,
                value,
                net_contributions: 11_000.0,
                realized_pnl: 0.0,
                unrealized_pnl: value - 11_000.0,
                is_complete: true,
            }
        })
        .collect()
}

fn data(count: usize, up: bool, range: PortfolioHistoryRange) -> CardData {
    let holdings = holdings(count);
    let moves: HashMap<String, AssetMove> = holdings
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let change = [
                0.184, 0.092, -0.061, 0.31, -0.142, 0.025, -0.02, 0.07, -0.3, 0.01,
            ][i];
            (
                h.asset.key.clone(),
                AssetMove {
                    start: 100.0,
                    latest: 100.0 * (1.0 + change),
                },
            )
        })
        .collect();
    let history = history(range, up);
    let total: f64 = holdings.iter().filter_map(|h| h.current_value).sum();
    let period_change = PortfolioPeriodChange::new(
        &history,
        total,
        history.last().map_or(0.0, |point| point.net_contributions),
    );
    CardData {
        has_portfolios: true,
        scope_name: "Main".to_owned(),
        performance: performance(&holdings, &moves),
        holdings,
        history,
        total_value: total,
        day_change: Some(PortfolioValueChange::new(total * 0.982, total)),
        period_change,
        formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
        loading: false,
        loading_history: false,
        error: None,
        unpriced: 0,
        loading_performance: false,
        now_ms: NOW,
        utc_offset_secs: 0,
    }
}

/// One card drawn the way the grid draws it: the surface frame, the header, the body.
fn card_at(
    ui: &mut egui::Ui,
    rect: Rect,
    kind: CardKind,
    range: PortfolioHistoryRange,
    data: Option<&CardData>,
    id: u64,
) {
    let mut card = PortfolioCard::new(kind, None);
    card.range = range;
    egui::Frame::new()
        .fill(theme::surface())
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(6))
        .stroke(egui::Stroke::new(1.0, theme::border()))
        .show(
            &mut ui.new_child(egui::UiBuilder::new().max_rect(rect)),
            |ui| {
                ui.set_min_size(rect.size() - vec2(14.0, 14.0));
                ui.set_max_size(rect.size() - vec2(14.0, 14.0));
                let scope = data.map_or("All Portfolios".to_owned(), |d| d.scope_name.clone());
                let mut action = ChartCardAction::default();
                header(ui, &card, &scope, id, false, &[], &mut action);
                body(ui, &mut card, data, id);
            },
        );
}

/// A card on a sheet: kind, range, where, its data and the pointer relative to the card.
type Placed = (
    CardKind,
    PortfolioHistoryRange,
    Rect,
    Option<CardData>,
    Option<Vec2>,
);

struct Sheet<'a> {
    name: &'a str,
    size: Vec2,
    /// `(kind, range, rect, data, pointer relative to the card)`.
    cards: Vec<Placed>,
}

fn render(sheet: &Sheet) {
    if gallery_dir().is_none() {
        return;
    }
    let _theme = theme::RestoreDefaultTheme;
    for (theme_name, app_theme) in [
        ("dark", theme::AppTheme::Dark),
        ("light", theme::AppTheme::Light),
    ] {
        let mut screen = Screen::new(sheet.size);
        egui_extras::install_image_loaders(&screen.ctx);
        theme::apply(&screen.ctx, app_theme);
        let pointer = sheet
            .cards
            .iter()
            .find_map(|(_, _, rect, _, p)| p.map(|p| rect.min + p));
        let mut last = None;
        for frame in 0..8 {
            let events = match pointer {
                Some(p) if frame > 0 => vec![egui::Event::PointerMoved(p)],
                _ => Vec::new(),
            };
            last = Some(screen.run(events, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, theme::background());
                for (index, (kind, range, rect, data, _)) in sheet.cards.iter().enumerate() {
                    card_at(ui, *rect, *kind, *range, data.as_ref(), index as u64 + 1);
                }
            }));
        }
        screen.save(
            &format!("{}-{theme_name}", sheet.name),
            last.expect("a frame"),
        );
    }
}

/// Cards of one kind at four sizes: wide, medium, narrow and tall.
fn sizes_sheet(name: &str, kind: CardKind, range: PortfolioHistoryRange, holds: usize) {
    let d = || Some(data(holds, true, range));
    let sizes = [
        vec2(560.0, 340.0),
        vec2(380.0, 300.0),
        vec2(270.0, 220.0),
        vec2(330.0, 480.0),
    ];
    let pad = 16.0;
    let mut cards = Vec::new();
    let mut x = pad;
    let mut y = pad;
    let mut row_height: f32 = 0.0;
    let width = 1000.0;
    for size in sizes {
        if x + size.x > width {
            x = pad;
            y += row_height + pad;
            row_height = 0.0;
        }
        cards.push((
            kind,
            range,
            Rect::from_min_size(Pos2::new(x, y), size),
            d(),
            None,
        ));
        x += size.x + pad;
        row_height = row_height.max(size.y);
    }
    render(&Sheet {
        name,
        size: vec2(width + pad, y + row_height + pad),
        cards,
    });
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_cards_gallery() {
    let month = PortfolioHistoryRange::OneMonth;
    sizes_sheet("card-value-chart", CardKind::ValueChart, month, 6);
    sizes_sheet("card-value", CardKind::Value, month, 6);
    sizes_sheet("card-allocation", CardKind::Allocation, month, 10);
    sizes_sheet("card-top-performers", CardKind::TopPerformers, month, 10);

    // Hover states.
    for (name, kind, hover) in [
        (
            "hover-value-chart",
            CardKind::ValueChart,
            vec2(260.0, 160.0),
        ),
        ("hover-allocation", CardKind::Allocation, vec2(150.0, 150.0)),
        (
            "hover-top-performers",
            CardKind::TopPerformers,
            vec2(200.0, 120.0),
        ),
    ] {
        let rect = Rect::from_min_size(Pos2::new(16.0, 16.0), vec2(560.0, 340.0));
        render(&Sheet {
            name,
            size: vec2(592.0, 372.0),
            cards: vec![(kind, month, rect, Some(data(10, true, month)), Some(hover))],
        });
    }

    // Loading, down, privacy and empty.
    let mut loading = data(6, true, month);
    loading.loading = true;
    let mut down = data(6, false, month);
    down.day_change = Some(PortfolioValueChange::new(
        down.total_value * 1.03,
        down.total_value,
    ));
    let mut private = data(6, true, month);
    private.formatter.privacy = true;
    let mut empty = data(0, true, month);
    empty.history.clear();
    empty.period_change = None;
    let mut unpriced = data(6, true, month);
    unpriced.unpriced = 2;
    unpriced.performance.unpriced = 2;
    let states = [loading, down, private, empty, unpriced];
    for (name, kind) in [
        ("states-value-chart", CardKind::ValueChart),
        ("states-value", CardKind::Value),
        ("states-allocation", CardKind::Allocation),
        ("states-top-performers", CardKind::TopPerformers),
    ] {
        let cards = states
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let x = 16.0 + (i % 3) as f32 * 396.0;
                let y = 16.0 + (i / 3) as f32 * 316.0;
                (
                    kind,
                    month,
                    Rect::from_min_size(Pos2::new(x, y), vec2(380.0, 300.0)),
                    Some(d.clone()),
                    None,
                )
            })
            .collect();
        render(&Sheet {
            name,
            size: vec2(1204.0, 632.0),
            cards,
        });
    }
}
