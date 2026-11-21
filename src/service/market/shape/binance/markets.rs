//! The market list of a venue, held for a day, and the search over it.

use super::Config;
use super::exchange_info::{Pair, matching_pairs, parse_pairs};
use crate::service::market::search_rank::normalize;
use crate::service::net::http;
use crate::service::{ProviderError, SearchResult, rank_results};

impl Config {
    /// Forgets the list held in memory (after the disk cache was cleared).
    pub(in crate::service) fn clear_catalog(&self) {
        self.catalog.clear_memory();
    }

    /// `BTC-USDT` and `btc/usdt` find `BTCUSDT`: the venue writes no separator.
    pub(in crate::service) fn search(
        &self,
        query: &str,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = self.catalog.get(|| self.download_pairs())?;
        let results = matching_pairs(self.source, &pairs, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }

    fn download_pairs(&self) -> Result<Vec<Pair>, ProviderError> {
        let body = http::get_text(self.exchange_info_url, &[], http::Retry::Transient)?;
        parse_pairs(&body, self.serves)
    }
}
