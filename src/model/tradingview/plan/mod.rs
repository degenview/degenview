//! What an import does: the person's choice of what to bring over and of which market each
//! TradingView symbol becomes ([`Mappings`]), turned into the layouts, watchlists and scripts to
//! add ([`Plan`]) and everything that is left out of them ([`LeftOut`]). Building a plan changes
//! nothing; the preview shows it and the host applies it.

mod layout;
mod mapping;

pub use mapping::{Mappings, Market, Match, Unmatched, Usage};

use super::library::{Library, WatchRow};
use crate::model::pine::script_name;
use crate::model::tradingview::TvSymbol;
use crate::model::tradingview::placement::Spot;
use crate::model::watchlist::{MAX_NAME_CHARS, Watchlist, WatchlistInstrument};
use crate::model::{ChartAppearance, ChartColumn, DataSource, PersistedChart, Timeframe};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The prices worked out for [`Selection::spots`]; `None` is a spot that has none.
pub type Placements = HashMap<Spot, Option<f64>>;

/// What to bring over.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub layouts: BTreeSet<String>,
    pub watchlists: BTreeSet<String>,
    pub scripts: BTreeSet<String>,
    /// A layout's timeframe when the person picked another than its first chart's.
    pub timeframes: BTreeMap<String, Timeframe>,
}

impl Selection {
    /// Everything found.
    pub fn all(library: &Library) -> Self {
        Self {
            layouts: library.layouts.iter().map(|l| l.id.clone()).collect(),
            watchlists: library.watchlists.iter().map(|l| l.id.clone()).collect(),
            scripts: library.scripts.iter().map(|s| s.id.clone()).collect(),
            timeframes: BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty() && self.watchlists.is_empty() && self.scripts.is_empty()
    }

    /// The choice for a library read again: what was left out of `before` stays out, and what is
    /// new comes in.
    pub fn carried_to(&self, before: &Library, after: &Library) -> Self {
        let mut next = Self::all(after);
        for layout in &before.layouts {
            if !self.layouts.contains(&layout.id) {
                next.layouts.remove(&layout.id);
            }
        }
        for list in &before.watchlists {
            if !self.watchlists.contains(&list.id) {
                next.watchlists.remove(&list.id);
            }
        }
        for script in &before.scripts {
            if !self.scripts.contains(&script.id) {
                next.scripts.remove(&script.id);
            }
        }
        next.timeframes.clone_from(&self.timeframes);
        next
    }

    /// Every market the chosen layouts and watchlists name, with what uses it, in the order they
    /// are met.
    /// The places the chosen layouts' drawings need worked out, now that markets are known.
    pub fn spots(&self, library: &Library, mappings: &Mappings) -> Vec<Spot> {
        library
            .layouts
            .iter()
            .filter(|layout| self.layouts.contains(&layout.id))
            .flat_map(|layout| &layout.drawings)
            .filter_map(|raw| Spot::of(raw, mappings.market(&raw.symbol)?))
            .collect()
    }

    pub fn symbols(&self, library: &Library) -> Vec<(TvSymbol, Usage)> {
        let mut order: Vec<TvSymbol> = Vec::new();
        let mut usage: BTreeMap<TvSymbol, Usage> = BTreeMap::new();
        let mut count = |symbol: &TvSymbol, add: fn(&mut Usage)| {
            if !usage.contains_key(symbol) {
                order.push(symbol.clone());
            }
            add(usage.entry(symbol.clone()).or_default());
        };
        for layout in &library.layouts {
            if !self.layouts.contains(&layout.id) {
                continue;
            }
            for chart in layout.shown_charts() {
                count(&chart.symbol, |u| u.charts += 1);
                for overlay in &chart.overlays {
                    count(overlay, |u| u.charts += 1);
                }
            }
            for drawing in &layout.drawings {
                count(&drawing.symbol, |u| u.drawings += 1);
            }
        }
        for list in &library.watchlists {
            if self.watchlists.contains(&list.id) {
                for symbol in list.symbols() {
                    count(symbol, |u| u.watchlists += 1);
                }
            }
        }
        order
            .into_iter()
            .map(|symbol| {
                let used = usage.get(&symbol).copied().unwrap_or_default();
                (symbol, used)
            })
            .collect()
    }
}

/// The names already in use, which an import never takes over.
#[derive(Clone, Debug, Default)]
pub struct Taken {
    pub layouts: Vec<String>,
    pub watchlists: Vec<String>,
    pub scripts: Vec<String>,
}

/// What an import added.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// `(id, name)` of each saved layout.
    pub layouts: Vec<(u64, String)>,
    pub drawings: usize,
    /// The names of the watchlists and scripts added.
    pub watchlists: Vec<String>,
    pub scripts: Vec<String>,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub layouts: Vec<PlannedLayout>,
    pub watchlists: Vec<PlannedWatchlist>,
    pub scripts: Vec<PlannedScript>,
}

impl Plan {
    pub fn drawing_count(&self) -> usize {
        self.layouts.iter().map(|layout| layout.drawings).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedLayout {
    pub source_id: String,
    pub name: String,
    pub timeframe: Timeframe,
    /// The timeframes TradingView's charts had, when they differ: the person can pick.
    pub timeframes: Vec<Timeframe>,
    pub charts: Vec<PersistedChart>,
    pub columns: Vec<ChartColumn>,
    /// Applied scripts of the account's own: `(chart id, instance id, TradingView script id)`.
    pub own_scripts: Vec<(u64, u64, String)>,
    pub drawings: usize,
    pub left_out: Vec<LeftOut>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedWatchlist {
    pub source_id: String,
    pub list: Watchlist,
    pub markets: usize,
    pub left_out: Vec<LeftOut>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedScript {
    pub source_id: String,
    pub name: String,
    pub source: String,
}

/// Something TradingView has that the import does not bring over, or changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeftOut {
    /// A layout none of whose charts has a market.
    NoChart,
    /// A chart whose market has no match.
    Chart(TvSymbol),
    /// The same, in a layout where no chart has one: the chart opens on the market most of the
    /// layout's drawings are on.
    StandIn {
        symbol: TvSymbol,
        source: DataSource,
        market: String,
    },
    /// A compared market with no match.
    Comparison(TvSymbol),
    /// Drawings on a market with no match.
    DrawingsOfMarket {
        symbol: TvSymbol,
        count: usize,
    },
    /// Drawings of a kind DegenView has no tool for.
    DrawingsOfTool {
        class: String,
        count: usize,
    },
    /// Pictures, which TradingView keeps on its servers.
    Pictures(usize),
    /// Positions whose distances cannot be worked out.
    PositionsWithoutPriceStep(usize),
    /// Drawings whose points do not make their shape.
    BrokenDrawings(usize),
    /// Drawings whose text was cut to DegenView's limit.
    CutText(usize),
    Indicator(String),
    /// An indicator is brought over with DegenView's settings, not TradingView's.
    IndicatorSettings(String),
    /// Charts TradingView keeps but its grid does not show.
    HiddenCharts(usize),
    /// A chart type DegenView does not draw.
    ChartStyle(i64),
    /// An interval that is not a length of time (ticks, ranges).
    Interval(String),
    /// A watchlist's markets with no match.
    Markets(usize),
    /// A watchlist none of whose symbols has a market.
    NoMarket,
    /// Symbols that became a market the list already has.
    SameMarket(usize),
    /// Symbols past what a watchlist holds.
    ListFull(usize),
    /// Fibonacci or Gann drawings brought over with DegenView's levels.
    DefaultLevels(usize),
    /// Drawings pinned to a place on the screen.
    Pinned(usize),
    /// Posts brought over at a height worked out from the candles around them.
    PostsEstimated(usize),
    /// Posts whose height is still being worked out.
    PostsPlacing(usize),
    /// Posts whose height could not be worked out.
    PostsUnplaced(usize),
    /// Anchored VWAPs whose percentage bands are off.
    PercentBands(usize),
    /// Drawings brought over onto a market their chart does not open on: they show once it is
    /// switched to that market.
    OtherMarket {
        source: DataSource,
        symbol: String,
        count: usize,
    },
}

impl LeftOut {
    /// Whether the thing still comes over, only not as it was, rather than not at all.
    pub fn is_change(&self) -> bool {
        match self {
            Self::StandIn { .. }
            | Self::CutText(_)
            | Self::IndicatorSettings(_)
            | Self::ChartStyle(_)
            | Self::Interval(_)
            | Self::SameMarket(_)
            | Self::DefaultLevels(_)
            | Self::OtherMarket { .. }
            | Self::PostsEstimated(_)
            | Self::PostsPlacing(_)
            | Self::PercentBands(_) => true,
            Self::NoChart
            | Self::Chart(_)
            | Self::Comparison(_)
            | Self::DrawingsOfMarket { .. }
            | Self::DrawingsOfTool { .. }
            | Self::Pictures(_)
            | Self::PositionsWithoutPriceStep(_)
            | Self::BrokenDrawings(_)
            | Self::Indicator(_)
            | Self::HiddenCharts(_)
            | Self::Markets(_)
            | Self::NoMarket
            | Self::ListFull(_)
            | Self::Pinned(_)
            | Self::PostsUnplaced(_) => false,
        }
    }
}

/// The plan for `selection`. `base` is the look a new chart gets here.
pub fn plan(
    library: &Library,
    selection: &Selection,
    mappings: &Mappings,
    placements: &Placements,
    taken: &Taken,
    base: &ChartAppearance,
) -> Plan {
    let mut script_names: HashSet<String> = taken
        .scripts
        .iter()
        .map(|name| script_name::key(name))
        .collect();
    let scripts: Vec<PlannedScript> = library
        .scripts
        .iter()
        .filter(|script| selection.scripts.contains(&script.id))
        .map(|script| {
            // Named as the library will name it.
            let base = script_name::portable(&script.name)
                .unwrap_or_else(|_| "TradingView script".to_owned());
            let name = script_name::numbered(&base, &script_names);
            script_names.insert(script_name::key(&name));
            PlannedScript {
                source_id: script.id.clone(),
                name,
                source: script.source.clone(),
            }
        })
        .collect();

    // The price step of a market, from whichever chart records it.
    let ticks: BTreeMap<&TvSymbol, f64> = library
        .layouts
        .iter()
        .flat_map(|layout| &layout.charts)
        .filter_map(|chart| Some((&chart.symbol, chart.tick?)))
        .collect();

    let mut layout_names = lower(&taken.layouts);
    let layouts = library
        .layouts
        .iter()
        .filter(|found| selection.layouts.contains(&found.id))
        .map(|found| {
            let mut planned = layout::plan(
                found,
                selection.timeframes.get(&found.id).copied(),
                mappings,
                placements,
                &ticks,
                library,
                base,
            );
            if !planned.charts.is_empty() {
                planned.name = unique(&mut layout_names, &planned.name);
            }
            planned
        })
        .collect();

    let mut list_names = lower(&taken.watchlists);
    let watchlists = library
        .watchlists
        .iter()
        .filter(|found| selection.watchlists.contains(&found.id))
        .map(|found| {
            let name = Watchlist::sanitized_name(&found.name);
            let name = if name.is_empty() {
                "TradingView".to_owned()
            } else {
                name
            };
            // Named once it is known to be added: a list with no market takes no name.
            let mut list = Watchlist::new(name.clone());
            let mut section = None;
            let mut markets = 0;
            let (mut missing, mut same, mut full) = (0, 0, 0);
            let mut seen = HashSet::new();
            for row in &found.rows {
                match row {
                    WatchRow::Section(title) => {
                        // An unnamed section's markets stay outside any, not in the one before.
                        let title = Watchlist::sanitized_name(title);
                        section = (!title.is_empty())
                            .then(|| list.add_section(&title, None).ok())
                            .flatten()
                            .map(|added| added.id);
                    }
                    WatchRow::Market(symbol) => match mappings.market(symbol) {
                        Some(market) => {
                            let mut item = WatchlistInstrument::from_market(
                                market.source,
                                &market.name,
                                &market.symbol,
                            );
                            item.image_url.clone_from(&market.image_url);
                            // Two TradingView venues may map to one market here.
                            if !seen.insert(item.instrument.key()) {
                                same += 1;
                            } else if list.add_checked(item, section).is_ok() {
                                markets += 1;
                            } else {
                                full += 1;
                            }
                        }
                        None => missing += 1,
                    },
                }
            }
            if markets > 0 {
                list.name = unique(&mut list_names, &name);
            }
            PlannedWatchlist {
                source_id: found.id.clone(),
                list,
                markets,
                left_out: [
                    (missing, LeftOut::Markets(missing)),
                    (same, LeftOut::SameMarket(same)),
                    (full, LeftOut::ListFull(full)),
                    (usize::from(markets == 0), LeftOut::NoMarket),
                ]
                .into_iter()
                .filter_map(|(count, entry)| (count > 0).then_some(entry))
                .collect(),
            }
        })
        .collect();

    Plan {
        layouts,
        watchlists,
        scripts,
    }
}

fn lower(names: &[String]) -> HashSet<String> {
    names.iter().map(|name| name.to_lowercase()).collect()
}

/// A layout's or watchlist's `name`, numbered until it is not in use, and then in use.
fn unique(used: &mut HashSet<String>, name: &str) -> String {
    let name: String = name.chars().take(MAX_NAME_CHARS - 6).collect();
    let mut candidate = name.clone();
    let mut number = 2;
    while !used.insert(candidate.to_lowercase()) {
        candidate = format!("{name} {number}");
        number += 1;
    }
    candidate
}

#[cfg(test)]
mod tests;
