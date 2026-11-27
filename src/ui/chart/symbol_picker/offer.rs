//! What can be done with a row, decided once. The footer's key hints, the preview's button and its
//! menu, the row's right-click menu, the keys and the row clicks all read the same [`Offer`], so
//! they cannot disagree.

use super::dashboards::{Tile, TilePick};
use super::{Env, Item, PickKind, PickMode, search};
use crate::model::InstrumentId;
use crate::model::portfolio::card::PortfolioCard;
use crate::service::SearchResult;

/// What Return does on a row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Primary {
    /// Show or hide an event's choices; `open` is how they are now.
    Fold {
        open: bool,
    },
    Replace,
    AddChart,
    AddCard,
    Compare,
    Watch,
}

/// Whether the row's market can be drawn over the focused chart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlay {
    Unavailable,
    Open,
    /// It is the chart's own market or already compared on it.
    Drawn,
}

/// What a row offers in the picker's current mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Offer {
    pub primary: Primary,
    /// Whether Return does anything: false when the market is already where it would go.
    pub enabled: bool,
    /// ⇧Return adds a chart, as something other than what Return does.
    pub add: bool,
    pub overlay: Overlay,
}

impl Offer {
    /// What `item` offers; `None` is a list with no row to highlight.
    pub(super) fn of(mode: PickMode, item: Option<&Item>, env: &Env) -> Self {
        let plain = |primary| Self {
            primary,
            enabled: true,
            add: false,
            overlay: Overlay::Unavailable,
        };
        if let Some(Item::Event(head)) = item {
            return plain(Primary::Fold { open: head.open });
        }
        let overlay = item.map_or(Overlay::Unavailable, |item| overlay_of(mode, item, env));
        match mode {
            PickMode::Watchlist => Self {
                enabled: !item.is_some_and(|item| holds(mode, item, env)),
                ..plain(Primary::Watch)
            },
            _ if item.and_then(card).is_some() => plain(Primary::AddCard),
            PickMode::Compare => Self {
                enabled: !item
                    .and_then(Item::market)
                    .is_some_and(|market| overlaid(env, market)),
                add: true,
                overlay,
                ..plain(Primary::Compare)
            },
            PickMode::Chart if env.replacing.is_some() => Self {
                add: true,
                overlay,
                ..plain(Primary::Replace)
            },
            PickMode::Chart | PickMode::Add => Self {
                overlay,
                ..plain(Primary::AddChart)
            },
        }
    }

    /// What else can be done besides what Return does, in menu order.
    pub fn others(&self) -> impl Iterator<Item = PickKind> {
        let add = self.add.then_some(PickKind::Add);
        let overlay = (self.overlay == Overlay::Open && self.primary != Primary::Compare)
            .then_some(PickKind::Overlay);
        add.into_iter().chain(overlay)
    }
}

/// Whether the row is already where a pick would put it: on the tab, or in the watchlist being
/// added to.
pub fn holds(mode: PickMode, item: &Item, env: &Env) -> bool {
    if matches!(item, Item::Event(_)) {
        return false;
    }
    match card(item) {
        Some(card) if mode == PickMode::Watchlist => holds_market(mode, &card.instrument_id(), env),
        Some(card) => env.cards_on_tab.iter().any(|on| on.same_card(card)),
        None => item.market().is_some_and(|result| {
            holds_market(
                mode,
                &InstrumentId::new(result.source, &result.full_symbol),
                env,
            )
        }),
    }
}

/// [`holds`] for a market by its identity.
pub fn holds_market(mode: PickMode, market: &InstrumentId, env: &Env) -> bool {
    let held = if mode == PickMode::Watchlist {
        env.in_list
    } else {
        env.on_tab
    };
    held.iter().any(|id| id.is_same_market(market))
}

/// Whether `result` is the focused chart's own market or one already compared on it.
pub fn overlaid(env: &Env, result: &SearchResult) -> bool {
    let market = InstrumentId::new(result.source, &result.full_symbol);
    env.overlaid.iter().any(|id| id.is_same_market(&market))
}

/// Whether the row's market can be drawn over the focused chart: only a crypto or stock market
/// can, over a chart that takes one.
pub fn overlay_of(mode: PickMode, item: &Item, env: &Env) -> Overlay {
    let market = item.market().filter(|_| !matches!(item, Item::Event(_)));
    match market {
        Some(market)
            if env.can_overlay
                && mode != PickMode::Watchlist
                && search::PROVIDERS.contains(&market.source) =>
        {
            if overlaid(env, market) {
                Overlay::Drawn
            } else {
                Overlay::Open
            }
        }
        _ => Overlay::Unavailable,
    }
}

fn card(item: &Item) -> Option<&PortfolioCard> {
    match item {
        Item::Tile(Tile {
            pick: TilePick::Card(card),
            ..
        }) => Some(card),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn item() -> Item {
        Item::Market(SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT"))
    }

    fn env<'a>(replacing: Option<&'a str>, held: &'a [InstrumentId]) -> Env<'a> {
        Env {
            on_tab: held,
            quote: None,
            replacing,
            overlaid: held,
            can_overlay: true,
            cards_on_tab: &[],
            in_list: held,
        }
    }

    fn others(mode: PickMode, env: &Env) -> Vec<PickKind> {
        Offer::of(mode, Some(&item()), env).others().collect()
    }

    #[test]
    fn each_mode_offers_its_own_return_and_the_rest_beside_it() {
        let chart = env(Some("ETHUSDT"), &[]);
        let offer = Offer::of(PickMode::Chart, Some(&item()), &chart);
        assert_eq!(offer.primary, Primary::Replace);
        assert_eq!(
            others(PickMode::Chart, &chart),
            [PickKind::Add, PickKind::Overlay]
        );
        // Adding is what Return does, so only comparing is left beside it.
        assert_eq!(others(PickMode::Add, &env(None, &[])), [PickKind::Overlay]);
        assert_eq!(others(PickMode::Compare, &chart), [PickKind::Add]);
        assert!(others(PickMode::Watchlist, &chart).is_empty());
    }

    #[test]
    fn a_market_already_there_leaves_nothing_to_do_twice() {
        let held = [InstrumentId::new(DataSource::Binance, "BTCUSDT")];
        let env = env(Some("ETHUSDT"), &held);
        for mode in [PickMode::Compare, PickMode::Watchlist] {
            assert!(!Offer::of(mode, Some(&item()), &env).enabled, "{mode:?}");
        }
        let chart = Offer::of(PickMode::Chart, Some(&item()), &env);
        assert_eq!(chart.overlay, Overlay::Drawn);
        assert!(chart.enabled);
        assert_eq!(others(PickMode::Chart, &env), [PickKind::Add]);
    }
}
