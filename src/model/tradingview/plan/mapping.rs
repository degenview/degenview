//! Which market each TradingView symbol becomes. The search proposes; the person's own choice
//! (another market, or none) stands over it until they reset it.

use crate::model::DataSource;
use crate::model::tradingview::TvSymbol;
use std::collections::BTreeMap;

/// A market of one of DegenView's providers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Market {
    pub source: DataSource,
    /// The symbol the provider is asked for.
    pub symbol: String,
    /// The name the provider shows.
    pub name: String,
    pub image_url: Option<String>,
}

/// Why a symbol has no market.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unmatched {
    /// A perpetual or a future.
    Derivative,
    /// No provider has the exchange, and the pair was not found elsewhere.
    Venue,
    NotFound,
    /// A formula, or something else that is not one market.
    Expression,
    /// A stock, with no Alpaca key to search with.
    NeedsKey,
    /// The providers could not be asked.
    Unreachable,
}

/// How a symbol stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Match {
    Searching,
    /// Found on its own exchange.
    Exact(Market),
    /// The same pair on another exchange.
    Substitute(Market),
    /// Picked by the person.
    Chosen(Market),
    /// Left out by the person.
    Skipped,
    Unmatched(Unmatched),
}

impl Match {
    pub fn market(&self) -> Option<&Market> {
        match self {
            Self::Exact(market) | Self::Substitute(market) | Self::Chosen(market) => Some(market),
            Self::Searching | Self::Skipped | Self::Unmatched(_) => None,
        }
    }
}

/// What uses a symbol in what is being imported.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub charts: usize,
    pub drawings: usize,
    pub watchlists: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Mappings {
    /// What the search said; absent while it has not answered.
    found: BTreeMap<TvSymbol, Match>,
    /// What the person decided.
    decided: BTreeMap<TvSymbol, Match>,
}

impl Mappings {
    pub fn status(&self, symbol: &TvSymbol) -> &Match {
        self.decided
            .get(symbol)
            .or_else(|| self.found.get(symbol))
            .unwrap_or(&Match::Searching)
    }

    pub fn market(&self, symbol: &TvSymbol) -> Option<&Market> {
        self.status(symbol).market()
    }

    /// The search answered; it never replaces the person's choice.
    pub fn found(&mut self, symbol: TvSymbol, result: Match) {
        self.found.insert(symbol, result);
    }

    pub fn has_answer(&self, symbol: &TvSymbol) -> bool {
        self.found.contains_key(symbol)
    }

    /// Takes the search's answer back, for it to be asked again.
    pub fn forget_found(&mut self, symbol: &TvSymbol) {
        self.found.remove(symbol);
    }

    pub fn choose(&mut self, symbol: &TvSymbol, market: Market) {
        self.decided.insert(symbol.clone(), Match::Chosen(market));
    }

    pub fn skip(&mut self, symbol: &TvSymbol) {
        self.decided.insert(symbol.clone(), Match::Skipped);
    }

    /// Takes the person's choice back.
    pub fn reset(&mut self, symbol: &TvSymbol) {
        self.decided.remove(symbol);
    }

    /// A fresh set that keeps what the person decided, for the same data read again.
    pub fn decisions(&self) -> Self {
        Self {
            found: BTreeMap::new(),
            decided: self.decided.clone(),
        }
    }

    pub fn is_decided(&self, symbol: &TvSymbol) -> bool {
        self.decided.contains_key(symbol)
    }
}
