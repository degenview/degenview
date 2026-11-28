//! The catalogs built from one [`MarketContext`](crate::service::market::ports::context::MarketContext),
//! so the lists they hold in memory can be forgotten together when the cache is cleared. A
//! catalog adds its memory here when it is built, so no venue lists its catalogs and none can be
//! left out; the registry belongs to the context, so a second context (a test's) has its own and
//! clearing one reaches only what was built from it.

use super::Loaded;
use crate::lock::lock;
use std::sync::{Arc, Mutex, Weak};

/// The memory of a catalog, whatever it lists.
pub(super) trait Held: Send + Sync {
    fn clear_memory(&self);
}

impl<T: Send + Sync + 'static> Held for Mutex<Option<Loaded<T>>> {
    fn clear_memory(&self) {
        *lock(self) = None;
    }
}

/// What the catalogs of one context hold in memory. A catalog that is gone drops out.
#[derive(Default)]
pub struct Registry {
    held: Mutex<Vec<Weak<dyn Held>>>,
}

impl Registry {
    pub(super) fn register(&self, memory: &Arc<dyn Held>) {
        let mut held = lock(&self.held);
        held.retain(|entry| entry.strong_count() > 0);
        held.push(Arc::downgrade(memory));
    }

    /// Forgets every list held in memory (after the disk cache was cleared), so the next search
    /// downloads it again.
    pub(in crate::service) fn clear_memory(&self) {
        for entry in lock(&self.held).iter() {
            if let Some(memory) = entry.upgrade() {
                memory.clear_memory();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Catalog;
    use super::*;
    use crate::model::{DataSource, Listing};
    use crate::service::market::ports::fakes::Rig;

    fn catalog(rig: &Rig, namespace: &'static str) -> Catalog<String> {
        Catalog::new(Listing::spot(DataSource::Bybit), namespace, &rig.context())
    }

    fn keep(catalog: &Catalog<String>) {
        catalog
            .get_for("all", || Ok(vec!["BTC".to_owned()]))
            .expect("a list");
        assert!(catalog.held("all").is_some());
    }

    #[test]
    fn clearing_a_contexts_registry_empties_every_catalog_built_from_it() {
        let rig = Rig::new();
        let (first, second, never_asked) = (
            catalog(&rig, "catalog_a"),
            catalog(&rig, "catalog_b"),
            catalog(&rig, "catalog_c"),
        );
        keep(&first);
        keep(&second);

        rig.catalogs.clear_memory();

        assert!(first.held("all").is_none());
        assert!(second.held("all").is_none());
        assert!(never_asked.held("all").is_none());
    }

    #[test]
    fn a_catalog_of_another_context_is_untouched() {
        let (ours, theirs) = (Rig::new(), Rig::new());
        let (mine, other) = (catalog(&ours, "catalog_a"), catalog(&theirs, "catalog_a"));
        keep(&mine);
        keep(&other);

        ours.catalogs.clear_memory();

        assert!(mine.held("all").is_none());
        assert!(other.held("all").is_some());
    }

    #[test]
    fn a_catalog_that_is_gone_is_forgotten_by_the_registry() {
        let rig = Rig::new();
        drop(catalog(&rig, "catalog_a"));
        let kept = catalog(&rig, "catalog_b");
        keep(&kept);
        assert_eq!(lock(&rig.catalogs.held).len(), 1, "the dropped one left");
        rig.catalogs.clear_memory();
        assert!(kept.held("all").is_none());
    }
}
