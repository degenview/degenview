//! The markets picked before, kept in the store so they can be picked again without searching.

use crate::model::DataSource;
use crate::service::SearchResult;
use serde::{Deserialize, Serialize};

/// Markets remembered as recent: the pickers and the new-tab page both list them, and both scroll.
pub(crate) const RECENT_LIMIT: usize = 24;

const RECENT_KEY: &str = "addTicker.recentMarkets";

/// A market picked before, kept so it can be picked again without searching. The price is
/// dropped, since it would be stale.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMarket {
    pub symbol: String,
    pub full_symbol: String,
    pub source: DataSource,
}

/// The markets picked before, newest first (shared with an empty tab's shortcuts).
pub fn load_recents(store: &crate::store::Store) -> Vec<RecentMarket> {
    store
        .setting::<Vec<RecentMarket>>(RECENT_KEY)
        .unwrap_or_default()
}

pub fn store_recents(store: &crate::store::Store, recents: &[RecentMarket]) {
    let _ = if recents.is_empty() {
        store.remove_setting(RECENT_KEY)
    } else {
        store.set_setting(RECENT_KEY, &recents)
    };
}

/// Puts `result` first in `recents`, the one place both sheets record a pick. Prediction markets
/// and CoinMarketCap indices are not kept; returns whether the list changed.
pub(crate) fn record_recent(recents: &mut Vec<RecentMarket>, result: &SearchResult) -> bool {
    if result.source.is_prediction_market() || result.source == DataSource::CoinMarketCap {
        return false;
    }
    recents.retain(|recent| {
        recent.source != result.source || recent.full_symbol != result.full_symbol
    });
    recents.insert(
        0,
        RecentMarket {
            symbol: result.symbol.clone(),
            full_symbol: result.full_symbol.clone(),
            source: result.source,
        },
    );
    recents.truncate(RECENT_LIMIT);
    true
}

impl RecentMarket {
    pub fn result(&self) -> SearchResult {
        SearchResult {
            symbol: self.symbol.clone(),
            full_symbol: self.full_symbol.clone(),
            source: self.source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }
}
