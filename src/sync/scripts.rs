//! User Pine scripts are files, not rows, so they reach sync through a small sink the app
//! implements over its `ScriptStore`. The sink is the only place that touches script files; the
//! apply and scan logic stays testable with an in-memory fake.
//!
//! Drafts and revisions never sync. Scripts the user never changed from the bundled examples are
//! never uploaded either, or every Mac would upload its own copy of the same nine examples.

use super::project::{Projected, ScriptSnapshot, project_script};
use crate::script_manager::demo::BUNDLED;
use crate::script_manager::store::{ScriptStore, with_trailing_newline};
use crate::store::Store;
use crate::store::sync::Reconciled;
use std::collections::{HashMap, HashSet};
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
    fn fresh_id(&mut self) -> String;
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
    fn fresh_id(&mut self) -> String {
        ScriptStore::fresh_id()
    }
}

/// The real sink over the app's `ScriptStore`. `unsaved` holds the ids with unsaved editor text
/// or a pending draft, supplied by the Script Manager.
pub struct StoreScriptSink<'a> {
    pub store: &'a mut ScriptStore,
    pub unsaved: HashSet<String>,
}

fn snapshot(script: &crate::script_manager::model::LocalScript) -> ScriptSnapshot {
    ScriptSnapshot {
        id: script.id.clone(),
        name: script.name.clone(),
        source: script.source.clone(),
        is_favorite: script.is_favorite,
    }
}

fn is_pristine_demo(name: &str, source: &str) -> bool {
    BUNDLED.iter().any(|demo| {
        demo.name.eq_ignore_ascii_case(name)
            && with_trailing_newline(demo.source) == with_trailing_newline(source)
    })
}

impl ScriptSink for StoreScriptSink<'_> {
    fn all(&mut self) -> Result<Vec<ScriptSnapshot>, String> {
        let scripts = self.store.all_scripts().map_err(|e| e.to_string())?;
        Ok(scripts
            .iter()
            .filter(|script| !is_pristine_demo(&script.name, &script.source))
            .map(snapshot)
            .collect())
    }

    fn get(&mut self, id: &str) -> Result<Option<ScriptSnapshot>, String> {
        Ok(self
            .store
            .script(id)
            .map_err(|e| e.to_string())?
            .as_ref()
            .map(snapshot))
    }

    fn find_by_name(&mut self, name: &str) -> Result<Option<NamedScript>, String> {
        let scripts = self.store.all_scripts().map_err(|e| e.to_string())?;
        Ok(scripts
            .iter()
            .find(|script| script.name.eq_ignore_ascii_case(name.trim()))
            .map(|script| NamedScript {
                id: script.id.clone(),
                pristine_demo: is_pristine_demo(&script.name, &script.source),
            }))
    }

    fn has_unsaved_work(&mut self, id: &str) -> bool {
        self.unsaved.contains(id) || self.store.draft(id).ok().flatten().is_some()
    }

    fn adopt(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String> {
        self.store
            .adopt(id, name, source, favorite)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn update(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String> {
        self.store
            .update_from_remote(id, name, source, favorite)
            .map_err(|e| e.to_string())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.store.delete(id).map_err(|e| e.to_string())
    }

    fn fresh_id(&mut self) -> String {
        ScriptStore::fresh_id()
    }
}
