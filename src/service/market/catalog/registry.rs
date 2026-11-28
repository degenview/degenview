//! The catalogs that hold a list in memory, so those lists can be forgotten together when the cache
//! is cleared. A catalog registers itself the first time it keeps a list, which is the only time it
//! has anything to forget; no list of venues is kept here.

use super::Catalog;
use crate::lock::lock;
use std::sync::Mutex;

/// A catalog, whatever it lists.
pub(super) trait Held: Sync {
    fn clear_memory(&self);
}

impl<T: Send + Sync> Held for Catalog<T> {
    fn clear_memory(&self) {
        Catalog::clear_memory(self);
    }
}

/// Each catalog once: it adds itself with its first list.
static HELD: Mutex<Vec<&'static dyn Held>> = Mutex::new(Vec::new());

pub(super) fn register(catalog: &'static dyn Held) {
    lock(&HELD).push(catalog);
}

/// Forgets every list held in memory (after the disk cache was cleared), so the next search
/// downloads it again.
pub(in crate::service) fn clear_memory() {
    for catalog in lock(&HELD).iter() {
        catalog.clear_memory();
    }
}
