//! A venue's search over its [`Catalog`](super::Catalog): the one way a list of markets is turned
//! into ranked hits. A venue says what a listed market is ([`ListedMarket`]) and where its list
//! comes from; the query, the matching, the ranking and the limit are written once here.

use crate::model::DataSource;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult, rank_results};
use std::sync::Arc;

/// How many hits one venue keeps once they are ranked, so the dollar markets are never cut off by
/// the odd quotes.
pub(in crate::service) const HITS: usize = 20;

/// A market in a venue's list, as far as search needs it.
pub(in crate::service) trait ListedMarket {
    /// The market as the venue names it (`BTC-USDT`, `BTCUSDT`, `EUR_USD`): what a hit carries and
    /// what a query is matched against, whatever it separates its parts with.
    fn id(&self) -> &str;

    /// The asset it trades and what it is priced in, as the venue lists them; `None` when the
    /// list does not say.
    fn sides(&self) -> Option<(&str, &str)>;

    /// What the venue calls the two sides, when it says.
    fn names(&self) -> (Option<&str>, Option<&str>) {
        (None, None)
    }
}

/// The best `limit` hits of `source` for `query`, from the list `list` brings. `BTC-USDT`,
/// `btc/usdt` and `BTCUSDT` are one query, and an empty one finds nothing without asking for the
/// list.
pub(in crate::service) fn find<M: ListedMarket>(
    source: DataSource,
    query: &str,
    limit: usize,
    list: impl FnOnce() -> Result<Arc<Vec<M>>, ProviderError>,
) -> Result<Vec<SearchResult>, ProviderError> {
    let needle = normalize(query);
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let markets = list()?;
    let hits = markets
        .iter()
        .filter(|market| normalize(market.id()).contains(&needle))
        .map(|market| hit(source, market))
        .collect();
    Ok(rank_results(hits, query).into_iter().take(limit).collect())
}

/// The names the venue gives the assets it lists, as `(upper-case ticker, name)`, from the list
/// `list` brings: none when it cannot be had.
pub(in crate::service) fn asset_names<M: ListedMarket>(
    list: impl FnOnce() -> Result<Arc<Vec<M>>, ProviderError>,
) -> Vec<(String, String)> {
    let Ok(markets) = list() else {
        return Vec::new();
    };
    markets
        .iter()
        .filter_map(|market| {
            let (base, _) = market.sides()?;
            Some((base.to_uppercase(), market.names().0?.to_owned()))
        })
        .collect()
}

fn hit(source: DataSource, market: &impl ListedMarket) -> SearchResult {
    let result = SearchResult::new(source, market.id(), market.id());
    let result = match market.sides() {
        Some((base, quote)) => result.pair(base, quote),
        None => result,
    };
    let (base_name, quote_name) = market.names();
    result.named(base_name, quote_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Listed {
        id: &'static str,
        base: &'static str,
        quote: &'static str,
        base_name: Option<&'static str>,
    }

    impl ListedMarket for Listed {
        fn id(&self) -> &str {
            self.id
        }

        fn sides(&self) -> Option<(&str, &str)> {
            Some((self.base, self.quote))
        }

        fn names(&self) -> (Option<&str>, Option<&str>) {
            (self.base_name, None)
        }
    }

    fn market(id: &'static str, base: &'static str, quote: &'static str) -> Listed {
        Listed {
            id,
            base,
            quote,
            base_name: None,
        }
    }

    fn list(markets: Vec<Listed>) -> impl FnOnce() -> Result<Arc<Vec<Listed>>, ProviderError> {
        move || Ok(Arc::new(markets))
    }

    fn symbols(hits: &[SearchResult]) -> Vec<&str> {
        hits.iter().map(|hit| hit.symbol.as_str()).collect()
    }

    #[test]
    fn a_market_is_found_however_the_query_separates_it() {
        let markets = || {
            vec![
                market("BTC-USDT", "BTC", "USDT"),
                market("ETH-BTC", "ETH", "BTC"),
            ]
        };
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = find(DataSource::Okx, query, HITS, list(markets())).unwrap();
            assert_eq!(symbols(&found), ["BTC-USDT"], "{query}");
        }
        let both = find(DataSource::Okx, "BTC", HITS, list(markets())).unwrap();
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn the_separator_the_venue_writes_does_not_matter_either() {
        let found = find(
            DataSource::Gate,
            "BTCUSDT",
            HITS,
            list(vec![market("BTC_USDT", "BTC", "USDT")]),
        )
        .unwrap();
        assert_eq!(symbols(&found), ["BTC_USDT"]);
    }

    #[test]
    fn an_empty_query_finds_nothing_and_does_not_ask_for_the_list() {
        for query in ["", "  ", "-/"] {
            let asked = Cell::new(false);
            let found = find(DataSource::Okx, query, HITS, || {
                asked.set(true);
                Ok(Arc::new(vec![market("BTC-USDT", "BTC", "USDT")]))
            })
            .unwrap();
            assert!(found.is_empty(), "{query:?}");
            assert!(!asked.get(), "{query:?}");
        }
    }

    #[test]
    fn a_list_that_cannot_be_had_is_the_search_error() {
        let error = find(DataSource::Okx, "btc", HITS, || {
            Err::<Arc<Vec<Listed>>, _>(ProviderError::other("offline"))
        })
        .unwrap_err();
        assert_eq!(error, ProviderError::other("offline"));
    }

    #[test]
    fn hits_are_ranked_as_the_picker_ranks_them() {
        // A market whose name only contains the query lists first in the venue's order.
        let markets = vec![
            market("WBTC-ETH", "WBTC", "ETH"),
            market("BTC-EUR", "BTC", "EUR"),
            market("BTC-USDT", "BTC", "USDT"),
        ];
        let found = find(DataSource::Okx, "btc", HITS, list(markets)).unwrap();
        assert_eq!(symbols(&found), ["BTC-USDT", "BTC-EUR", "WBTC-ETH"]);
    }

    #[test]
    fn a_search_keeps_the_best_hits_up_to_the_limit() {
        let ranked = |limit| {
            let markets = vec![
                market("BTC-EUR", "BTC", "EUR"),
                market("BTC-USDT", "BTC", "USDT"),
                market("BTC-USD", "BTC", "USD"),
            ];
            let found = find(DataSource::Okx, "btc", limit, list(markets)).unwrap();
            found
                .iter()
                .map(|hit| hit.symbol.clone())
                .collect::<Vec<_>>()
        };
        // OKX's main dollar quote is USDT.
        assert_eq!(ranked(HITS), ["BTC-USDT", "BTC-USD", "BTC-EUR"]);
        // The cut falls on the worst, not on the venue's first.
        assert_eq!(ranked(2), ["BTC-USDT", "BTC-USD"]);
        assert_eq!(ranked(0), Vec::<String>::new());
    }

    #[test]
    fn a_hit_carries_the_venue_its_sides_and_its_names() {
        let named = Listed {
            id: "BTC_USDT",
            base: "BTC",
            quote: "USDT",
            base_name: Some("Bitcoin"),
        };
        let found = find(DataSource::Gate, "btc", HITS, list(vec![named])).unwrap();
        let hit = &found[0];
        assert_eq!(hit.source, DataSource::Gate);
        assert_eq!(
            (hit.symbol.as_str(), hit.full_symbol.as_str()),
            ("BTC_USDT", "BTC_USDT")
        );
        assert_eq!(hit.base.as_deref(), Some("BTC"));
        assert_eq!(hit.quote.as_deref(), Some("USDT"));
        assert_eq!(hit.names.base.as_deref(), Some("Bitcoin"));
        assert_eq!(hit.names.quote, None);
    }

    #[test]
    fn a_market_that_does_not_say_its_sides_is_a_hit_without_them() {
        struct Bare;
        impl ListedMarket for Bare {
            fn id(&self) -> &str {
                "GOLD"
            }

            fn sides(&self) -> Option<(&str, &str)> {
                None
            }
        }
        let found = find(DataSource::Oanda, "gold", HITS, || Ok(Arc::new(vec![Bare]))).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].base.as_deref(), found[0].quote.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn asset_names_are_the_listed_ones_under_their_upper_case_ticker() {
        let markets = vec![
            Listed {
                id: "btc_usdt",
                base: "btc",
                quote: "usdt",
                base_name: Some("Bitcoin"),
            },
            market("ETH_USDT", "ETH", "USDT"),
        ];
        assert_eq!(
            asset_names(list(markets)),
            [("BTC".to_owned(), "Bitcoin".to_owned())]
        );
    }

    #[test]
    fn asset_names_are_none_when_the_list_cannot_be_had() {
        let names = asset_names(|| Err::<Arc<Vec<Listed>>, _>(ProviderError::other("offline")));
        assert!(names.is_empty());
    }
}
