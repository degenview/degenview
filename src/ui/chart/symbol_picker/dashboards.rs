//! The Dashboards tab: CoinMarketCap's market-wide indices and views of the user's portfolios.
//! Nothing is searched; the query filters what is listed.

use super::columns::{Columns, ICON, PAD};
use super::list::{
    ROW_HEIGHT, RowOutput, RowState, marks, paint_background, two_lines, venue_block, venue_lines,
};
use crate::i18n::tr;
use crate::model::DataSource;
use crate::model::market::cmc::CHART_TYPES;
use crate::model::portfolio::Portfolio;
use crate::model::portfolio::card::{CardKind, PortfolioCard};
use crate::service::SearchResult;
use crate::service::market::search_rank::normalize;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{Rect, Response, Sense, Ui, Vec2, pos2, vec2};

/// Icon and description per entry of [`CHART_TYPES`], in the same order.
fn gauge_details() -> [(Icon, &'static str); 4] {
    [
        (Icon::ChartLine, tr!("Historical 0–100 index chart")),
        (
            Icon::GaugeHigh,
            tr!("Current Bitcoin/Altcoin season reading"),
        ),
        (Icon::Waveform, tr!("Historical market-sentiment chart")),
        (Icon::Gauge, tr!("Current sentiment gauge")),
    ]
}

/// What a tile adds to the tab.
#[derive(Clone, Debug, PartialEq)]
pub enum TilePick {
    /// A CoinMarketCap index, which replaces or adds a chart like any market.
    Gauge(SearchResult),
    /// A portfolio view, which only ever adds a card.
    Card(PortfolioCard),
}

/// A row of the tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub icon: Icon,
    pub title: String,
    pub summary: String,
    /// The right-hand caption of a card: which portfolio it covers.
    pub scope: Option<String>,
    pub pick: TilePick,
}

/// A caption and the tiles under it.
pub struct Group {
    pub heading: &'static str,
    pub tiles: Vec<Tile>,
}

/// The groups for `query`: the indices, then the cards of `scope` (every portfolio when `None`).
/// Without portfolios there are no cards to offer, which the tab says itself.
pub fn groups(portfolios: &[Portfolio], scope: Option<u64>, query: &str) -> Vec<Group> {
    let needle = normalize(query);
    let matches = |tile: &Tile| {
        needle.is_empty()
            || normalize(&tile.title).contains(&needle)
            || normalize(&tile.summary).contains(&needle)
    };
    let gauges = CHART_TYPES
        .iter()
        .zip(gauge_details())
        .map(|((key, title), (icon, summary))| Tile {
            icon,
            title: (*title).to_owned(),
            summary: summary.to_owned(),
            scope: None,
            pick: TilePick::Gauge(SearchResult::new(DataSource::CoinMarketCap, *title, *key)),
        })
        .filter(matches)
        .collect();
    let scope = scope.filter(|id| portfolios.iter().any(|portfolio| portfolio.id == *id));
    let card_scope = PortfolioCard::new(CardKind::Value, scope).scope_name(portfolios);
    let cards = if portfolios.is_empty() {
        Vec::new()
    } else {
        CardKind::ALL
            .into_iter()
            .map(|kind| Tile {
                icon: crate::ui::portfolio::cards::icon(kind),
                title: kind.display_title().to_owned(),
                summary: kind.summary().to_owned(),
                scope: Some(card_scope.clone()),
                pick: TilePick::Card(PortfolioCard::new(kind, scope)),
            })
            .filter(matches)
            .collect()
    };
    [(tr!("Market indices"), gauges), (tr!("Portfolio"), cards)]
        .into_iter()
        .filter(|(_, tiles): &(&str, Vec<Tile>)| !tiles.is_empty())
        .map(|(heading, tiles)| Group { heading, tiles })
        .collect()
}

/// What a tile adds to the tab: a bare icon, its title over what it shows, and where it comes
/// from at the right.
pub fn row(ui: &mut Ui, tile: &Tile, state: RowState, columns: &Columns) -> (Response, RowOutput) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, tile.title.clone());
    let mut out = RowOutput {
        hovered: response.hovered(),
        ..RowOutput::default()
    };
    if !ui.is_rect_visible(rect) {
        return (response, out);
    }
    paint_background(ui, rect, state.highlighted, response.hovered());
    // A tile is not a market: nothing to compare, and its own check says it is already added.
    let anchor = match &tile.pick {
        TilePick::Gauge(result) => result.clone(),
        TilePick::Card(card) => {
            SearchResult::new(DataSource::CoinMarketCap, &tile.title, card.key())
        }
    };
    marks(ui, rect, &anchor, state, &mut out);
    out.pick |= response.clicked() && !out.add;

    let icon = Rect::from_center_size(
        pos2(rect.left() + PAD + ICON / 2.0, rect.center().y),
        Vec2::splat(20.0),
    );
    icons::paint(ui.painter(), icon, tile.icon, theme::text_secondary());
    match &tile.pick {
        TilePick::Gauge(result) => venue_block(ui, rect, columns, result.source),
        // A card's venue column names the portfolio it covers.
        TilePick::Card(_) => {
            venue_lines(
                ui,
                rect,
                columns,
                tile.scope.as_deref().unwrap_or_default(),
                tr!("Portfolio"),
            );
        }
    }
    two_lines(
        ui,
        rect,
        rect.left() + columns.symbol,
        columns.title_width(),
        &tile.title,
        &tile.summary,
    );
    (response, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portfolio(id: u64, name: &str) -> Portfolio {
        Portfolio::new(id, name, 0)
    }

    fn titles(groups: &[Group]) -> Vec<Vec<&str>> {
        groups
            .iter()
            .map(|group| group.tiles.iter().map(|tile| tile.title.as_str()).collect())
            .collect()
    }

    #[test]
    fn the_indices_come_first_then_the_cards_of_every_portfolio() {
        let groups = groups(&[portfolio(1, "Main")], None, "");
        assert_eq!(
            groups.iter().map(|group| group.heading).collect::<Vec<_>>(),
            ["Market indices", "Portfolio"]
        );
        assert_eq!(groups[0].tiles.len(), CHART_TYPES.len());
        assert_eq!(groups[1].tiles.len(), CardKind::ALL.len());
        assert_eq!(groups[1].tiles[0].scope.as_deref(), Some("All Portfolios"));
    }

    #[test]
    fn a_portfolio_scopes_the_cards_and_names_itself() {
        let list = [portfolio(1, "Main"), portfolio(2, "Alts")];
        let groups = groups(&list, Some(2), "");
        let TilePick::Card(card) = &groups[1].tiles[0].pick else {
            panic!("a card");
        };
        assert_eq!(card.portfolio_id, Some(2));
        assert_eq!(groups[1].tiles[0].scope.as_deref(), Some("Alts"));
        // A portfolio that no longer exists falls back to all of them.
        let gone = super::groups(&list, Some(9), "");
        let TilePick::Card(card) = &gone[1].tiles[0].pick else {
            panic!("a card");
        };
        assert_eq!(card.portfolio_id, None);
    }

    #[test]
    fn with_no_portfolio_there_are_no_cards() {
        let groups = groups(&[], None, "");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].heading, "Market indices");
    }

    #[test]
    fn the_query_filters_titles_and_summaries_and_drops_empty_groups() {
        let list = [portfolio(1, "Main")];
        let fear = groups(&list, None, "fear");
        assert_eq!(
            titles(&fear),
            [vec!["Fear & Greed History", "Fear & Greed Index"]]
        );
        let split = groups(&list, None, "splits");
        assert_eq!(titles(&split), [vec!["Allocation"]]);
        assert!(groups(&list, None, "zzz").is_empty());
    }

    #[test]
    fn a_gauge_is_the_market_the_chart_asks_for() {
        let groups = groups(&[], None, "season index");
        let TilePick::Gauge(result) = &groups[0].tiles[0].pick else {
            panic!("a gauge");
        };
        assert_eq!(result.source, DataSource::CoinMarketCap);
        assert_eq!(result.symbol, "Altcoin Season Index");
        assert_eq!(result.full_symbol, CHART_TYPES[1].0);
    }
}
