use crate::model::InstrumentId;
use crate::model::watchlist::quote::WatchlistQuote;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Latest quotes by `InstrumentId::key()`. Nothing here is persisted. A cheap handle:
/// clones share the same quotes, so the coordinator writes and the UI reads.
#[derive(Clone, Default)]
pub struct WatchlistQuoteBook {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    quotes: RwLock<HashMap<String, WatchlistQuote>>,
    revision: AtomicU64,
}

impl WatchlistQuoteBook {
    pub fn get(&self, instrument: &InstrumentId) -> Option<WatchlistQuote> {
        self.get_by_key(&instrument.key())
    }

    pub fn get_by_key(&self, key: &str) -> Option<WatchlistQuote> {
        self.inner.quotes.read().ok()?.get(key).cloned()
    }

    /// Bumps whenever any quote changes, so a sorted list knows when to re-sort.
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
    }

    /// Forgets quotes nobody asks for any more. Returns whether any was held.
    pub fn remove(&self, keys: &[String]) -> bool {
        let Ok(mut quotes) = self.inner.quotes.write() else {
            return false;
        };
        let before = quotes.len();
        for key in keys {
            quotes.remove(key);
        }
        let changed = quotes.len() != before;
        if changed {
            self.inner.revision.fetch_add(1, Ordering::AcqRel);
        }
        changed
    }

    /// Replaces quotes for the given keys; an unchanged quote does not count as a change.
    /// Returns whether anything changed.
    pub fn apply(&self, updates: HashMap<String, WatchlistQuote>) -> bool {
        let Ok(mut quotes) = self.inner.quotes.write() else {
            return false;
        };
        let mut changed = false;
        for (key, quote) in updates {
            if quotes.get(&key) != Some(&quote) {
                quotes.insert(key, quote);
                changed = true;
            }
        }
        if changed {
            self.inner.revision.fetch_add(1, Ordering::AcqRel);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::watchlist::quote::Freshness;

    fn quote(last: f64) -> WatchlistQuote {
        WatchlistQuote {
            last: Some(last),
            freshness: Freshness::Live,
            ..WatchlistQuote::default()
        }
    }

    #[test]
    fn clones_share_quotes_and_unchanged_writes_do_not_bump_the_revision() {
        let book = WatchlistQuoteBook::default();
        let reader = book.clone();
        let id = InstrumentId::new(DataSource::Binance, "btcusdt");
        assert!(reader.get(&id).is_none());

        let update = || HashMap::from([(id.key(), quote(1.0))]);
        assert!(book.apply(update()));
        assert_eq!(reader.get(&id).and_then(|quote| quote.last), Some(1.0));
        assert_eq!(reader.revision(), 1);

        assert!(!book.apply(update()), "same quote again is not a change");
        assert_eq!(reader.revision(), 1);

        assert!(book.apply(HashMap::from([(id.key(), quote(2.0))])));
        assert_eq!(reader.revision(), 2);
    }

    #[test]
    fn removing_forgets_quotes_and_bumps_the_revision_only_when_one_was_held() {
        let book = WatchlistQuoteBook::default();
        let id = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        book.apply(HashMap::from([(id.key(), quote(1.0))]));
        let revision = book.revision();
        assert!(!book.remove(&["BINANCE:ETHUSDT".to_owned()]));
        assert_eq!(book.revision(), revision);
        assert!(book.remove(&[id.key()]));
        assert!(book.get(&id).is_none());
        assert_eq!(book.revision(), revision + 1);
    }

    #[test]
    fn lookup_is_case_insensitive_where_the_provider_is() {
        let book = WatchlistQuoteBook::default();
        book.apply(HashMap::from([(
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key(),
            quote(5.0),
        )]));
        assert!(
            book.get(&InstrumentId::new(DataSource::Binance, "btcusdt"))
                .is_some()
        );
    }
}
