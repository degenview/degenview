//! The importable things found in TradingView's data, in TradingView's own terms.

use super::TvSymbol;
use super::drawing::RawDrawing;
use super::grid::Grid;

/// Everything found, newest first within each list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    pub layouts: Vec<Layout>,
    pub watchlists: Vec<TvWatchlist>,
    pub scripts: Vec<Script>,
}

impl Library {
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty() && self.watchlists.is_empty() && self.scripts.is_empty()
    }

    pub fn contents(&self) -> Contents {
        let layouts = &self.layouts;
        let watchlists = &self.watchlists;
        let scripts = &self.scripts;
        Contents {
            layouts: layouts.iter().map(Layout::display_name).collect(),
            drawings: layouts.iter().map(|layout| layout.drawings.len()).sum(),
            watchlists: watchlists.iter().map(|list| list.name.clone()).collect(),
            markets: watchlists.iter().map(|list| list.symbols().count()).sum(),
            scripts: scripts.iter().map(|script| script.name.clone()).collect(),
            lines: scripts
                .iter()
                .map(|script| script.source.lines().count())
                .sum(),
        }
    }
}

/// What a library holds, in outline: each kind's names and how much is in them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Contents {
    pub layouts: Vec<String>,
    /// Over all layouts.
    pub drawings: usize,
    pub watchlists: Vec<String>,
    /// Over all watchlists.
    pub markets: usize,
    pub scripts: Vec<String>,
    /// Over all scripts.
    pub lines: usize,
}

/// A saved chart layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// TradingView's short id (the end of the chart's address).
    pub id: String,
    /// The name given in TradingView; `None` when only the layout's drawings were found.
    pub name: Option<String>,
    pub grid: Grid,
    /// Every chart the layout stores, the shown ones first. Never empty.
    pub charts: Vec<LayoutChart>,
    pub drawings: Vec<RawDrawing>,
    /// Drawings whose newest state could not be read.
    pub unreadable: usize,
    /// When TradingView last fetched any of it, in epoch milliseconds.
    pub seen_at_ms: i64,
}

impl Layout {
    /// TradingView's name for it, or one made of its id when only its drawings were found.
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("TradingView {}", self.id))
    }

    /// The layout's settings were not found: its charts are rebuilt from its drawings.
    pub fn is_rebuilt(&self) -> bool {
        self.name.is_none()
    }

    pub fn shown_charts(&self) -> impl Iterator<Item = &LayoutChart> {
        self.charts.iter().filter(|chart| chart.shown)
    }
}

pub(crate) fn push_unique(list: &mut Vec<TvSymbol>, symbol: &TvSymbol) {
    if !list.contains(symbol) {
        list.push(symbol.clone());
    }
}

/// One chart of a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutChart {
    pub symbol: TvSymbol,
    /// TradingView's resolution string (`5`, `60`, `1D`).
    pub interval: String,
    /// TradingView's series style number; `None` when only drawings were found.
    pub style: Option<i64>,
    /// Markets compared on the chart.
    pub overlays: Vec<TvSymbol>,
    pub studies: Vec<Study>,
    /// The market's smallest price step, when the layout records it.
    pub tick: Option<f64>,
    /// Whether the layout's grid shows it. TradingView keeps the charts of a bigger grid after
    /// switching to a smaller one.
    pub shown: bool,
}

/// An indicator on a chart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Study {
    /// TradingView's id without the version (`RSI@tv-basicstudies`), or a user script's id.
    pub id: String,
    pub name: String,
    /// Its `length` input, when it has one.
    pub length: Option<u32>,
}

/// A watchlist: its markets under their section titles, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TvWatchlist {
    pub id: String,
    pub name: String,
    pub rows: Vec<WatchRow>,
}

impl TvWatchlist {
    pub fn symbols(&self) -> impl Iterator<Item = &TvSymbol> {
        self.rows.iter().filter_map(|row| match row {
            WatchRow::Market(symbol) => Some(symbol),
            WatchRow::Section(_) => None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchRow {
    Section(String),
    Market(TvSymbol),
}

/// A Pine script the account's owner wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub source: String,
    /// TradingView's version of the newest copy found (`25.0`).
    pub version: String,
}
