//! What a chart tab shows before it has a chart (Swift `EmptyStateView`): a search launcher and,
//! under it, tabs of one-click ways into a market: recently opened ones, favorites, suggestions
//! for someone starting out, and saved layouts. Behind the top of the page a trend line keeps
//! moving.

mod backdrop;
mod sparkline;
mod tabs;
mod tiles;

pub use sparkline::Trend;
pub use tabs::Tab;

use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, InstrumentId};
use crate::ui::chart::add::sheet::RecentMarket;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use std::collections::HashMap;

/// Widest the tabs' content grows.
const CONTENT_WIDTH: f32 = 1000.0;
/// Widest the title and search box grow.
const HEADER_WIDTH: f32 = 760.0;
const LAUNCHER_HEIGHT: f32 = 50.0;
/// How far the title, everything under it and the trend line sit below the top of the page.
const PAGE_TOP: f32 = 48.0;
/// Chips shown per tab; the rest stays reachable from the sidebar or Add Chart.
pub const ITEM_LIMIT: usize = 8;

/// One saved layout as its card shows it.
pub struct LayoutSummary {
    pub id: u64,
    pub name: String,
    pub chart_count: usize,
    pub timeframe: &'static str,
    /// The first few markets, as `(source, symbol)`.
    pub previews: Vec<(DataSource, String)>,
}

pub struct EmptyStateInput<'a> {
    /// Sidebar order, already capped at `ITEM_LIMIT`.
    pub favorites: &'a [WatchlistInstrument],
    /// Favorites that did not fit.
    pub hidden_favorites: usize,
    /// Newest first, every one remembered.
    pub recents: &'a [RecentMarket],
    pub layouts: &'a [LayoutSummary],
    /// The last week of markets that have been read, by `InstrumentId::key()`. A market that is
    /// not in it is still loading.
    pub trends: &'a HashMap<String, Trend>,
}

/// What the page remembers between frames.
#[derive(Default)]
pub struct EmptyStateState {
    /// The tab the person picked; until then the page opens on [`tabs::default_tab`].
    pub tab: Option<Tab>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmptyStateAction {
    AddChart,
    OpenFavorite(usize),
    OpenRecent(usize),
    OpenLayout(u64),
    /// A starter market: `(source, provider symbol, display symbol)`.
    OpenSuggestion(DataSource, String, String),
    RemoveRecent(usize),
    ClearRecents,
    ShowFavorites,
}

pub struct EmptyStateOutput {
    pub actions: Vec<EmptyStateAction>,
    /// The markets whose tiles were drawn, so the caller can read a week of closes for them.
    pub wanted: Vec<InstrumentId>,
}

pub fn show(ui: &mut Ui, input: &EmptyStateInput, state: &mut EmptyStateState) -> EmptyStateOutput {
    let area = ui.max_rect();
    backdrop::show(
        ui,
        Rect::from_min_max(pos2(area.left(), area.top() + PAGE_TOP), area.max),
    );
    let mut frame = tiles::Frame::new(input.trends, area.bottom());
    let mut tab = state.tab.unwrap_or_else(|| tabs::default_tab(input));
    egui::ScrollArea::vertical()
        .id_salt("empty-state")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Below the trend line, lower in a tall window.
            ui.add_space(
                PAGE_TOP
                    + (area.height() * 0.12)
                        .clamp(backdrop::LINE_TOP + backdrop::LINE_RANGE, 180.0),
            );
            centred(ui, (area.width() - 48.0).min(CONTENT_WIDTH), |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                centred(ui, HEADER_WIDTH, |ui| {
                    hero(ui, !input.recents.is_empty());
                    ui.add_space(26.0);
                    if launcher(ui).clicked() {
                        frame.actions.push(EmptyStateAction::AddChart);
                    }
                });
                ui.add_space(24.0);
                let items = Tab::ALL.map(|tab| (tab, tab.title(), tab.icon()));
                if let Some(picked) = kit::icon_tabs::icon_tabs(ui, &items, tab) {
                    state.tab = Some(picked);
                    tab = picked;
                }
                ui.add_space(20.0);
                tabs::show(ui, tab, input, &mut frame);
            });
            ui.add_space(36.0);
        });
    EmptyStateOutput {
        actions: frame.actions,
        wanted: frame.wanted,
    }
}

/// Runs `add` in a column `width` wide (less when there is no room) in the middle of `ui`.
fn centred<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = width.min(ui.available_width());
    let left = ui.cursor().left() + (ui.available_width() - width) / 2.0;
    let room = Rect::from_min_size(
        pos2(left, ui.cursor().top()),
        vec2(width, ui.available_height().min(10_000.0)),
    );
    ui.scope_builder(UiBuilder::new().max_rect(room), add).inner
}

fn hero(ui: &mut Ui, has_history: bool) {
    centred_line(
        ui,
        "Start a new chart",
        size::TITLE2 + 11.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    ui.add_space(8.0);
    centred_line(
        ui,
        if has_history {
            "Search a market, or pick up where you left off."
        } else {
            "Search crypto, stocks and prediction markets."
        },
        size::BODY + 2.0,
        REGULAR,
        theme::text_secondary(),
    );
}

fn centred_line(ui: &mut Ui, text: &str, font: f32, weight: f32, color: Color32) {
    let galley = kit::galley(ui, text, font, weight, color, ui.available_width());
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top()),
        galley,
        color,
    );
}

/// A search box that is really a button: it opens Add Chart, where the typing happens.
fn launcher(ui: &mut Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), LAUNCHER_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, "Add Chart")
        .on_hover_text("Add Chart")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    tiles::surface(ui, rect, rect.height() / 2.0, hovered);
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), Vec2::splat(17.0)),
        Icon::Magnifier,
        if hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    // What it finds, as quiet marks at the right.
    let mut x = rect.right() - 28.0;
    for icon in [Icon::Percent, Icon::BuildingColumns, Icon::BitcoinCircle] {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(x, rect.center().y), Vec2::splat(16.0)),
            icon,
            theme::text_tertiary(),
        );
        x -= 28.0;
    }
    painter.vline(
        x + 7.0,
        rect.center().y - 9.0..=rect.center().y + 9.0,
        egui::Stroke::new(1.0, kit::hairline()),
    );
    let prompt = kit::galley(
        ui,
        "Search crypto, stocks, prediction markets…",
        size::BODY + 2.0,
        REGULAR,
        theme::text_secondary(),
        (x - rect.left() - 80.0).max(40.0),
    );
    ui.painter().galley(
        pos2(rect.left() + 56.0, rect.center().y - prompt.size().y / 2.0),
        prompt,
        theme::text_secondary(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favorite(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument {
            id: 1,
            instrument: InstrumentId::new(DataSource::Binance, symbol),
            name: "Bitcoin".into(),
            label: "BTC".into(),
            display_name: None,
            image_url: None,
            card: None,
        }
    }

    fn recent(symbol: &str) -> RecentMarket {
        RecentMarket {
            symbol: symbol.into(),
            full_symbol: symbol.into(),
            source: DataSource::Binance,
        }
    }

    fn layout() -> LayoutSummary {
        LayoutSummary {
            id: 3,
            name: "Majors".into(),
            chart_count: 2,
            timeframe: "1D",
            previews: vec![(DataSource::Binance, "BTCUSDT".into())],
        }
    }

    #[test]
    fn every_tab_draws_headlessly_with_and_without_data() {
        let favorites = [favorite("BTCUSDT")];
        let recents = [recent("ETHUSDT")];
        let layouts = [layout()];
        let mut trends = HashMap::new();
        trends.insert(
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key(),
            Trend::Ready(vec![1.0, 2.0, 1.5, 3.0]),
        );
        trends.insert(
            InstrumentId::new(DataSource::Binance, "ETHUSDT").key(),
            Trend::Unavailable,
        );
        let ctx = egui::Context::default();
        for returning in [false, true] {
            for tab in Tab::ALL {
                let input = EmptyStateInput {
                    favorites: if returning { &favorites } else { &[] },
                    hidden_favorites: usize::from(returning) * 2,
                    recents: if returning { &recents } else { &[] },
                    layouts: if returning { &layouts } else { &[] },
                    trends: &trends,
                };
                let mut state = EmptyStateState { tab: Some(tab) };
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let page = show(ui, &input, &mut state);
                    assert!(page.actions.is_empty());
                    let shows_markets = matches!(tab, Tab::Suggestions)
                        || (returning && matches!(tab, Tab::Recent | Tab::Favorites));
                    assert_eq!(
                        !page.wanted.is_empty(),
                        shows_markets,
                        "{tab:?}, returning {returning}"
                    );
                });
                output.textures_delta.clear();
            }
        }
    }

    /// The markets whose tiles one frame of a 1000 x 700 page drew.
    fn wanted(input: &EmptyStateInput, tab: Tab) -> usize {
        let ctx = egui::Context::default();
        let mut state = EmptyStateState { tab: Some(tab) };
        let mut count = 0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            count = show(ui, input, &mut state).wanted.len();
        });
        output.textures_delta.clear();
        count
    }

    #[test]
    fn suggestions_always_include_stocks() {
        let trends = HashMap::new();
        let input = EmptyStateInput {
            favorites: &[],
            hidden_favorites: 0,
            recents: &[],
            layouts: &[],
            trends: &trends,
        };
        assert_eq!(wanted(&input, Tab::Suggestions), 20);
    }

    #[test]
    fn a_long_recent_list_draws_and_asks_for_only_the_rows_in_view() {
        let trends = HashMap::new();
        let recents: Vec<RecentMarket> = (0..24).map(|n| recent(&format!("C{n}USDT"))).collect();
        let input = EmptyStateInput {
            favorites: &[],
            hidden_favorites: 0,
            recents: &recents,
            layouts: &[],
            trends: &trends,
        };
        let drawn = wanted(&input, Tab::Recent);
        assert!((3..24).contains(&drawn), "{drawn} rows drawn of 24");
    }

    #[test]
    fn capped_favorites_end_in_a_tile_that_opens_the_rest() {
        let trends = HashMap::new();
        let favorites = [favorite("BTCUSDT"), favorite("ETHUSDT")];
        let input = |hidden| EmptyStateInput {
            favorites: &favorites,
            hidden_favorites: hidden,
            recents: &[],
            layouts: &[],
            trends: &trends,
        };
        // The "more" tile is not a market, so it asks for no week.
        assert_eq!(wanted(&input(3), Tab::Favorites), 2);
        assert_eq!(wanted(&input(0), Tab::Favorites), 2);
    }

    /// Pictures of the page for a person to look at. Run by hand:
    /// `DEGENVIEW_GALLERY_DIR=… cargo test --lib empty_state -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_page() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::AppTheme;
        if gallery_dir().is_none() {
            return;
        }
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        let favorites: Vec<WatchlistInstrument> = ["BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT"]
            .into_iter()
            .enumerate()
            .map(|(index, symbol)| WatchlistInstrument {
                id: index as _,
                ..favorite(symbol)
            })
            .collect();
        let recents: Vec<RecentMarket> = (0..24)
            .map(|n| recent(["DOGEUSDT", "AVAXUSDT", "LINKUSDT", "XRPUSDT"][n % 4]))
            .collect();
        let layouts = [
            layout(),
            LayoutSummary {
                id: 2,
                name: "Tech stocks".into(),
                chart_count: 1,
                timeframe: "4H",
                previews: vec![(DataSource::Alpaca, "NVDA".into())],
            },
        ];
        // A week for every market on the page: some climbing, some falling.
        let mut trends = HashMap::new();
        let symbols = [
            "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK",
        ];
        for (index, base) in symbols.iter().enumerate() {
            let slope = if index % 4 == 1 { -0.6 } else { 1.0 };
            let week: Vec<f32> = (0..7)
                .map(|day| 10.0 + slope * day as f32 + ((day * 5 + index * 3) % 4) as f32 * 0.7)
                .collect();
            for symbol in [format!("{base}USDT"), (*base).to_owned()] {
                trends.insert(
                    InstrumentId::new(DataSource::Binance, symbol).key(),
                    Trend::Ready(week.clone()),
                );
            }
        }
        for (index, ticker) in [
            "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD",
        ]
        .into_iter()
        .enumerate()
        {
            let slope = if index % 3 == 1 { -0.5 } else { 0.8 };
            let week: Vec<f32> = (0..7)
                .map(|day| 50.0 + slope * day as f32 + ((day * 3 + index) % 4) as f32 * 0.6)
                .collect();
            trends.insert(
                InstrumentId::new(DataSource::Alpaca, ticker).key(),
                Trend::Ready(week),
            );
        }
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for (width, size_name) in [(1100.0, "wide"), (640.0, "narrow")] {
                for (returning, tab) in [
                    (false, Tab::Suggestions),
                    (true, Tab::Recent),
                    (true, Tab::Favorites),
                    (true, Tab::Layouts),
                    (false, Tab::Recent),
                ] {
                    let mut screen = Screen::new(egui::vec2(width, 760.0));
                    crate::ui::theme::apply(&screen.ctx, app_theme);
                    let input = EmptyStateInput {
                        favorites: if returning { &favorites } else { &[] },
                        hidden_favorites: usize::from(returning) * 4,
                        recents: if returning { &recents } else { &[] },
                        layouts: if returning { &layouts } else { &[] },
                        trends: &trends,
                    };
                    let mut state = EmptyStateState { tab: Some(tab) };
                    let mut last = None;
                    for _ in 0..4 {
                        last = Some(screen.run(Vec::new(), |ui| {
                            show(ui, &input, &mut state);
                        }));
                    }
                    screen.save(
                        &format!(
                            "empty-page-{}-{}-{size_name}-{theme_name}",
                            tab.title().to_lowercase(),
                            if returning { "data" } else { "none" },
                        ),
                        last.expect("a frame"),
                    );
                }
            }
        }
    }
}
