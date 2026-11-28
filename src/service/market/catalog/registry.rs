//! Every venue's [`Catalog`], so the lists held in memory can be forgotten together when the cache
//! is cleared. The one list of them: a venue that declares a catalog is added here, and a test reads
//! the sources and fails for a catalog this list lacks.

use super::Catalog;
use crate::service::market::{alpaca, binance, bybit, coinbase, gateio, kraken, mexc, oanda, okx};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A catalog, whatever it lists.
trait Held: Sync {
    /// Where the catalog keeps its list in the lookup cache.
    #[cfg(test)]
    fn namespace(&self) -> &'static str;

    fn clear_memory(&self);
}

impl<T: Serialize + DeserializeOwned + Send + Sync> Held for Catalog<T> {
    #[cfg(test)]
    fn namespace(&self) -> &'static str {
        self.namespace
    }

    fn clear_memory(&self) {
        Catalog::clear_memory(self);
    }
}

static CATALOGS: [&dyn Held; 10] = [
    &binance::SPOT_CATALOG,
    &binance::PERPETUAL_CATALOG,
    &coinbase::CATALOG,
    &okx::CATALOG,
    &kraken::CATALOG,
    &bybit::CATALOG,
    &gateio::CATALOG,
    &mexc::CATALOG,
    &alpaca::CATALOG,
    &oanda::CATALOG,
];

/// Forgets every list held in memory (after the disk cache was cleared), so the next search
/// downloads it again.
pub(in crate::service) fn clear_memory() {
    clear(&CATALOGS);
}

fn clear(catalogs: &[&dyn Held]) {
    for catalog in catalogs {
        catalog.clear_memory();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::cache::disk::{DiskCache, testing::TempCache};
    use crate::service::cache::lookup::LookupCache;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    /// The namespace of every `Catalog::new("…")` in the venues' code (tests and comments left out).
    fn declared_namespaces() -> BTreeSet<String> {
        const DECLARATION: &str = "Catalog::new(\"";
        let mut namespaces = BTreeSet::new();
        let mut pending = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service/market")];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).expect("directory readable").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs")
                    && path.file_name().is_some_and(|name| name != "tests.rs")
                {
                    let source = fs::read_to_string(&path).expect("source readable");
                    let code = source.split("#[cfg(test)]").next().unwrap_or_default();
                    for line in code
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                    {
                        let mut rest = line;
                        while let Some((_, after)) = rest.split_once(DECLARATION) {
                            let (namespace, tail) = after.split_once('"').expect("a closed name");
                            namespaces.insert(namespace.to_owned());
                            rest = tail;
                        }
                    }
                }
            }
        }
        namespaces
    }

    #[test]
    fn every_catalog_a_venue_declares_is_registered() {
        let registered: Vec<&str> = CATALOGS.iter().map(|catalog| catalog.namespace()).collect();
        let distinct: BTreeSet<&str> = registered.iter().copied().collect();
        assert_eq!(distinct.len(), registered.len(), "a catalog listed twice");
        let declared = declared_namespaces();
        assert!(
            !declared.is_empty(),
            "no declaration found: has the scan gone stale?"
        );
        assert_eq!(
            distinct,
            declared.iter().map(String::as_str).collect(),
            "a Catalog::new in market/ and the registry differ"
        );
    }

    #[test]
    fn clearing_reaches_every_catalog_it_is_given() {
        let temp = TempCache::new();
        let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(temp.path())));
        let cache = LookupCache::for_tests(disk, Box::new(|| 0));
        let catalogs = ["catalog_a", "catalog_b", "catalog_c"].map(Catalog::<String>::new);
        for catalog in &catalogs {
            catalog
                .get_from(&cache, "all", || Ok(vec!["BTC".to_owned()]))
                .expect("a list");
            assert!(catalog.held("all").is_some());
        }
        clear(&catalogs.each_ref().map(|catalog| catalog as &dyn Held));
        for catalog in &catalogs {
            assert!(catalog.held("all").is_none(), "{}", catalog.namespace);
        }
    }
}
