//! Queues the user's script files for upload: the scan that compares the library with what was
//! last synced. The sink and the record shapes are `sync::record::scripts`.

use crate::store::Store;
use crate::store::sync::Reconciled;
use crate::sync::record::policy::EntityKind;
use crate::sync::record::project::{Projected, project_script};
use crate::sync::record::scripts::{ScriptCache, ScriptSink};
use std::collections::HashMap;

/// Re-reads the library, refreshes the cache the sync thread uploads from, and queues whatever
/// changed: new scripts, edits, renames, favorites, deletions. Run debounced after a script save,
/// when the app becomes active, and periodically (this also catches edits made in Finder).
/// A library that cannot be listed changes nothing: an I/O error must never read as "deleted".
pub fn scan_scripts(
    store: &Store,
    sink: &mut dyn ScriptSink,
    cache: &ScriptCache,
) -> rusqlite::Result<Reconciled> {
    let Ok(scripts) = sink.all() else {
        return Ok(Reconciled::default());
    };
    let current: HashMap<String, Projected> = scripts
        .iter()
        .map(|script| (script.id.clone(), project_script(script)))
        .collect();
    if let Ok(mut shared) = cache.lock() {
        *shared = current.clone();
    }
    store.reconcile_kind(EntityKind::Script, &current)
}
