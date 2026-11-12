//! The empty page a tab shows before it has a chart, and the slow reads behind it.

mod trends;

use self::trends::TrendLoader;
use super::ChartWorkspace;
use crate::model::DataSource;
use crate::model::watchlist::WatchlistInstrument;
use crate::service::SearchResult;
use crate::ui::chart::add::sheet;
use crate::ui::workspace::empty_state::{self, EmptyStateAction, EmptyStateState};
use eframe::egui;
use std::time::Duration;

/// The slow inputs of the empty page: the recents setting, every saved layout and whether
/// Alpaca is configured (a Keychain read). Reloaded when the page first shows and when the
/// window regains focus, as Swift's `onAppear` and `didBecomeKey` do, so a layout saved in
/// another tab appears without a read on every repaint.
#[derive(Default)]
pub(super) struct EmptyPageCache {
    pub(super) data: Option<EmptyPageData>,
    was_focused: bool,
    /// When the page last drew. A tab behind another in its window is not drawn, so a long gap
    /// means it was just brought to the front (a frame is requested every 250 ms while it shows).
    last_drawn: Option<std::time::Instant>,
    /// The tab the person picked, until a chart replaces the page.
    state: EmptyStateState,
    /// The week of closes behind each tile's sparkline.
    trends: TrendLoader,
}

impl EmptyPageCache {
    /// Forgets the slow reads and the picked tab, so the next visit starts from fresh data.
    pub(super) fn reset(&mut self) {
        self.data = None;
        self.state = EmptyStateState::default();
    }
}

/// Longer than the repaint interval, shorter than any deliberate tab switch.
const EMPTY_PAGE_REVEAL_GAP: Duration = Duration::from_millis(900);

pub(super) struct EmptyPageData {
    recents: Vec<sheet::RecentMarket>,
    layouts: Vec<empty_state::LayoutSummary>,
}

impl ChartWorkspace {
    /// An empty tab: a search launcher and, in tabs, shortcuts back into recent markets,
    /// favorites and saved layouts, or starting points.
    pub(super) fn show_empty_state(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
        let revealed = self
            .empty_page
            .last_drawn
            .is_none_or(|drawn| drawn.elapsed() > EMPTY_PAGE_REVEAL_GAP);
        self.empty_page.last_drawn = Some(std::time::Instant::now());
        let regained_focus = (focused && !self.empty_page.was_focused) || revealed;
        self.empty_page.was_focused = focused;
        let mut page = match self.empty_page.data.take() {
            Some(page) if !regained_focus => page,
            _ => self.load_empty_page(),
        };

        // Favorites are in memory and follow the sidebar live.
        let favorites_list: Vec<WatchlistInstrument> = self
            .watchlist
            .lock()
            .map(|store| {
                let id = store.favorites();
                store
                    .lists()
                    .iter()
                    .find(|list| list.id == id)
                    .map(|list| list.instruments().cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let hidden_favorites = favorites_list.len().saturating_sub(empty_state::ITEM_LIMIT);
        let favorites: Vec<WatchlistInstrument> = favorites_list
            .into_iter()
            .take(empty_state::ITEM_LIMIT)
            .collect();
        let trends = self.empty_page.trends.snapshot();
        let input = empty_state::EmptyStateInput {
            favorites: &favorites,
            hidden_favorites,
            recents: &page.recents,
            layouts: &page.layouts,
            trends: &trends,
        };
        let output = empty_state::show(ui, &input, &mut self.empty_page.state);
        self.empty_page.trends.request(&output.wanted, &ctx);
        for action in output.actions {
            match action {
                EmptyStateAction::AddChart => self.add_chart.open(),
                EmptyStateAction::OpenFavorite(index) => {
                    self.add_chart_from_instrument(&favorites[index], &ctx);
                }
                EmptyStateAction::OpenRecent(index) => {
                    self.add_from_search(&page.recents[index].result(), &ctx);
                }
                EmptyStateAction::OpenLayout(id) => self.request_open_saved_view(id, &ctx),
                EmptyStateAction::OpenSuggestion(source, symbol, display) => {
                    self.add_from_search(
                        &SearchResult {
                            symbol: display,
                            full_symbol: symbol,
                            source,
                            price: None,
                            event: None,
                            base: None,
                            quote: None,
                        },
                        &ctx,
                    );
                }
                EmptyStateAction::RemoveRecent(index) => {
                    page.recents.remove(index);
                    self.store_recents(&page.recents);
                }
                EmptyStateAction::ClearRecents => {
                    page.recents.clear();
                    self.store_recents(&page.recents);
                }
                EmptyStateAction::ShowFavorites => self.watchlist_visible = true,
            }
        }
        self.empty_page.data = Some(page);
    }

    fn load_empty_page(&self) -> EmptyPageData {
        let recents = self
            .store
            .lock()
            .map(|store| sheet::load_recents(&store))
            .unwrap_or_default();
        let mut views = self.load_saved_views();
        views.sort_by_key(|view| std::cmp::Reverse(view.created_at_ms));
        let layouts = views
            .iter()
            .map(|view| empty_state::LayoutSummary {
                id: view.id,
                name: view.name.clone(),
                chart_count: view.charts.len(),
                timeframe: view.timeframe.label(),
                previews: view
                    .charts
                    .iter()
                    .filter(|chart| {
                        chart.card.is_none() && chart.source != DataSource::CoinMarketCap
                    })
                    .map(|chart| (chart.source, chart.symbol.clone()))
                    .collect(),
            })
            .collect();
        EmptyPageData { recents, layouts }
    }

    fn store_recents(&self, recents: &[sheet::RecentMarket]) {
        if let Ok(store) = self.store.lock() {
            sheet::store_recents(&store, recents);
        }
    }
}
