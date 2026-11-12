//! The page's four tabs and what each shows.

use super::EmptyStateAction;
use super::EmptyStateInput;
use super::tiles::{self, Frame, Market};
use crate::model::DataSource;
use crate::ui::icons::Icon;
use crate::ui::kit;
use eframe::egui::{Id, Ui};

/// Most columns of tiles; five fill the page's width the way the markets are named.
const MAX_COLUMNS: usize = 5;
const MARKET_TILE_WIDTH: f32 = 184.0;
const MARKET_TILE_HEIGHT: f32 = 64.0;
const LAYOUT_CARD_WIDTH: f32 = 230.0;
const LAYOUT_CARD_HEIGHT: f32 = 72.0;
const ROW_HEIGHT: f32 = 58.0;
/// Under the recent list: the gap, the "Clear" link and the page's bottom margin.
const CLEAR_ROOM: f32 = 70.0;
/// Rows the recent list leaves out of the room under it, and the fewest it shows.
const SPARE_ROWS: f32 = 4.0;
const MIN_ROWS: f32 = 3.0;

/// Starter crypto markets: ticker and name.
const CRYPTO: [(&str, &str); 10] = [
    ("BTC", "Bitcoin"),
    ("ETH", "Ethereum"),
    ("SOL", "Solana"),
    ("BNB", "BNB"),
    ("XRP", "XRP"),
    ("ADA", "Cardano"),
    ("DOGE", "Dogecoin"),
    ("AVAX", "Avalanche"),
    ("DOT", "Polkadot"),
    ("LINK", "Chainlink"),
];
/// Starter stocks and ETFs: ticker and name.
const STOCKS: [(&str, &str); 10] = [
    ("AAPL", "Apple"),
    ("MSFT", "Microsoft"),
    ("NVDA", "NVIDIA"),
    ("AMZN", "Amazon"),
    ("GOOGL", "Alphabet"),
    ("META", "Meta"),
    ("TSLA", "Tesla"),
    ("SPY", "S&P 500"),
    ("QQQ", "Nasdaq 100"),
    ("AMD", "AMD"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Recent,
    Favorites,
    Suggestions,
    Layouts,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Recent, Tab::Favorites, Tab::Suggestions, Tab::Layouts];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Recent => "Recent",
            Tab::Favorites => "Favorites",
            Tab::Suggestions => "Suggestions",
            Tab::Layouts => "Layouts",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Tab::Recent => Icon::ClockArrow,
            Tab::Favorites => Icon::Star,
            Tab::Suggestions => Icon::Sparkles,
            Tab::Layouts => Icon::Books,
        }
    }
}

/// The tab a page opens on: the first of Recent, Favorites and Layouts that has something in it,
/// and for someone with nothing yet, Suggestions.
pub fn default_tab(input: &EmptyStateInput) -> Tab {
    if !input.recents.is_empty() {
        Tab::Recent
    } else if !input.favorites.is_empty() {
        Tab::Favorites
    } else if !input.layouts.is_empty() {
        Tab::Layouts
    } else {
        Tab::Suggestions
    }
}

pub fn show(ui: &mut Ui, tab: Tab, input: &EmptyStateInput, frame: &mut Frame) {
    match tab {
        Tab::Recent => recent(ui, input, frame),
        Tab::Favorites => favorites(ui, input, frame),
        Tab::Suggestions => suggestions(ui, frame),
        Tab::Layouts => layouts(ui, input, frame),
    }
}

fn recent(ui: &mut Ui, input: &EmptyStateInput, frame: &mut Frame) {
    if input.recents.is_empty() {
        tiles::note(ui, Icon::ClockArrow, "Markets you open will show up here.");
        return;
    }
    // The list takes the room the page has left under it, less a few rows, never fewer than
    // `MIN_ROWS`: a short list is easier to take in, and it scrolls for the rest.
    let room = (frame.page_bottom - ui.cursor().top() - CLEAR_ROOM - SPARE_ROWS * ROW_HEIGHT)
        .max(MIN_ROWS * ROW_HEIGHT);
    tiles::list_card(
        ui,
        input.recents.len(),
        ROW_HEIGHT,
        room,
        |ui, index, rect| {
            let market = &input.recents[index];
            let response = tiles::market_row(
                ui,
                rect,
                Id::new(("empty-recent", index)),
                Market {
                    icon_symbol: &market.symbol,
                    api_symbol: &market.full_symbol,
                    source: market.source,
                    title: &market.symbol,
                    subtitle: Some(market.source.display_name()),
                    badged: true,
                },
                frame,
            );
            kit::fitted_context_menu(ui, &response, &[("Remove from Recent", None)], |ui| {
                if kit::menu_item(ui, Icon::Trash, "Remove from Recent", true, false) {
                    frame.actions.push(EmptyStateAction::RemoveRecent(index));
                    ui.close();
                }
            });
            if response.clicked() {
                frame.actions.push(EmptyStateAction::OpenRecent(index));
            }
        },
    );
    ui.add_space(12.0);
    if tiles::link_row(ui, "Clear") {
        frame.actions.push(EmptyStateAction::ClearRecents);
    }
}

fn favorites(ui: &mut Ui, input: &EmptyStateInput, frame: &mut Frame) {
    if input.favorites.is_empty() {
        tiles::note(
            ui,
            Icon::Star,
            "Star a market in the watchlist and it will show up here.",
        );
        return;
    }
    let more = input.hidden_favorites > 0;
    tiles::grid(
        ui,
        input.favorites.len() + usize::from(more),
        MARKET_TILE_WIDTH,
        MAX_COLUMNS,
        MARKET_TILE_HEIGHT,
        |ui, index, rect| {
            let Some(item) = input.favorites.get(index) else {
                if tiles::more_tile(ui, rect, input.hidden_favorites).clicked() {
                    frame.actions.push(EmptyStateAction::ShowFavorites);
                }
                return;
            };
            if tiles::market_tile(
                ui,
                rect,
                Id::new(("empty-favorite", item.id)),
                Market {
                    icon_symbol: &item.label,
                    api_symbol: &item.instrument.api_symbol(),
                    source: item.instrument.source,
                    title: &item.label,
                    subtitle: Some(&item.name),
                    badged: true,
                },
                frame,
            )
            .clicked()
            {
                frame.actions.push(EmptyStateAction::OpenFavorite(index));
            }
        },
    );
}

fn suggestions(ui: &mut Ui, frame: &mut Frame) {
    starters(ui, &CRYPTO, DataSource::Binance, "empty-crypto", frame);
    tiles::separator(ui);
    starters(ui, &STOCKS, DataSource::Alpaca, "empty-stock", frame);
}

/// A grid of starter markets from one provider: crypto is quoted in USDT, stocks by ticker.
fn starters(
    ui: &mut Ui,
    markets: &[(&str, &str)],
    source: DataSource,
    id: &str,
    frame: &mut Frame,
) {
    tiles::grid(
        ui,
        markets.len(),
        MARKET_TILE_WIDTH,
        MAX_COLUMNS,
        MARKET_TILE_HEIGHT,
        |ui, index, rect| {
            let (ticker, name) = markets[index];
            let symbol = if source == DataSource::Binance {
                format!("{ticker}USDT")
            } else {
                ticker.to_owned()
            };
            let display = if source == DataSource::Binance {
                format!("{ticker}/USDT")
            } else {
                ticker.to_owned()
            };
            if tiles::market_tile(
                ui,
                rect,
                Id::new((id, ticker)),
                Market {
                    icon_symbol: ticker,
                    api_symbol: &symbol,
                    source,
                    title: ticker,
                    subtitle: Some(name),
                    badged: false,
                },
                frame,
            )
            .clicked()
            {
                frame
                    .actions
                    .push(EmptyStateAction::OpenSuggestion(source, symbol, display));
            }
        },
    );
}

fn layouts(ui: &mut Ui, input: &EmptyStateInput, frame: &mut Frame) {
    if input.layouts.is_empty() {
        tiles::note(
            ui,
            Icon::Books,
            "Save a layout from the layout menu and it will show up here.",
        );
        return;
    }
    tiles::grid(
        ui,
        input.layouts.len(),
        LAYOUT_CARD_WIDTH,
        MAX_COLUMNS,
        LAYOUT_CARD_HEIGHT,
        |ui, index, rect| {
            if tiles::layout_card(ui, rect, &input.layouts[index]).clicked() {
                frame
                    .actions
                    .push(EmptyStateAction::OpenLayout(input.layouts[index].id));
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InstrumentId;
    use crate::model::watchlist::WatchlistInstrument;
    use crate::ui::chart::add::sheet::RecentMarket;
    use crate::ui::workspace::empty_state::LayoutSummary;
    use std::collections::HashMap;

    fn favorite() -> WatchlistInstrument {
        WatchlistInstrument {
            id: 1,
            instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            name: "Bitcoin".into(),
            label: "BTC".into(),
            display_name: None,
            image_url: None,
            card: None,
        }
    }

    fn recent() -> RecentMarket {
        RecentMarket {
            symbol: "ETHUSDT".into(),
            full_symbol: "ETHUSDT".into(),
            source: DataSource::Binance,
        }
    }

    fn layout() -> LayoutSummary {
        LayoutSummary {
            id: 1,
            name: "Majors".into(),
            chart_count: 2,
            timeframe: "1D",
            previews: Vec::new(),
        }
    }

    fn tab_for(has_recent: bool, has_favorite: bool, has_layout: bool) -> Tab {
        let (favorites, recents, layouts) = (
            if has_favorite {
                vec![favorite()]
            } else {
                vec![]
            },
            if has_recent { vec![recent()] } else { vec![] },
            if has_layout { vec![layout()] } else { vec![] },
        );
        default_tab(&EmptyStateInput {
            favorites: &favorites,
            hidden_favorites: 0,
            recents: &recents,
            layouts: &layouts,
            trends: &HashMap::new(),
        })
    }

    #[test]
    fn the_page_opens_on_the_first_tab_with_something_in_it() {
        assert_eq!(tab_for(false, false, false), Tab::Suggestions);
        assert_eq!(tab_for(true, true, true), Tab::Recent);
        assert_eq!(tab_for(false, true, true), Tab::Favorites);
        assert_eq!(tab_for(false, false, true), Tab::Layouts);
        assert_eq!(tab_for(true, false, false), Tab::Recent);
    }

    #[test]
    fn the_tabs_are_in_the_order_the_page_shows_them() {
        assert_eq!(
            Tab::ALL.map(Tab::title),
            ["Recent", "Favorites", "Suggestions", "Layouts"]
        );
    }
}
