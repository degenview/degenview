//! User Pine scripts are files, not rows, so they reach sync through a small sink the app
//! implements over its `ScriptStore` (`host::sync::scripts`). The sink is the only place that
//! touches script files; the apply and scan logic stays testable with an in-memory fake.
//!
//! Drafts and revisions never sync. Scripts the user never changed from the bundled examples are
//! never uploaded either, or every Mac would upload its own copy of the same nine examples.

use super::project::{Projected, ScriptSnapshot, project_script};
use crate::store::Store;
use crate::store::sync::Reconciled;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A script found by name, for deciding whether an incoming script may replace it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedScript {
    pub id: String,
    /// Still exactly the bundled example it was seeded as.
    pub pristine_demo: bool,
}

pub trait ScriptSink {
    /// Every script that syncs: all of them except unmodified bundled examples.
    fn all(&mut self) -> Result<Vec<ScriptSnapshot>, String>;
    fn get(&mut self, id: &str) -> Result<Option<ScriptSnapshot>, String>;
    /// A script with this name, ignoring case.
    fn find_by_name(&mut self, name: &str) -> Result<Option<NamedScript>, String>;
    /// Text the user has typed but not saved. Sync never overwrites it.
    fn has_unsaved_work(&mut self, id: &str) -> bool;
    fn adopt(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String>;
    fn update(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String>;
    /// Moves the script to the Trash.
    fn delete(&mut self, id: &str) -> Result<(), String>;
}

/// The scripts as the sync thread may send them, refreshed by the scanner on the app thread.
pub type ScriptCache = Arc<Mutex<HashMap<String, Projected>>>;

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
    store.reconcile_kind(super::policy::EntityKind::Script, &current)
}

/// For builds without a script library (tests, other platforms).
pub struct NoScripts;

impl ScriptSink for NoScripts {
    fn all(&mut self) -> Result<Vec<ScriptSnapshot>, String> {
        Ok(Vec::new())
    }
    fn get(&mut self, _: &str) -> Result<Option<ScriptSnapshot>, String> {
        Ok(None)
    }
    fn find_by_name(&mut self, _: &str) -> Result<Option<NamedScript>, String> {
        Ok(None)
    }
    fn has_unsaved_work(&mut self, _: &str) -> bool {
        false
    }
    fn adopt(&mut self, _: &str, _: &str, _: &str, _: bool) -> Result<(), String> {
        Err("no script library".into())
    }
    fn update(&mut self, _: &str, _: &str, _: &str, _: bool) -> Result<(), String> {
        Err("no script library".into())
    }
    fn delete(&mut self, _: &str) -> Result<(), String> {
        Err("no script library".into())
    }
}
