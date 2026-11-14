//! The sync sink over the app's script files: what `sync::record::scripts` asks of the library,
//! answered from the `ScriptStore`.

use crate::model::pine::script::LocalScript;
use crate::pine_studio::library::demo::BUNDLED;
use crate::store::scripts::{ScriptStore, with_trailing_newline};
use crate::sync::record::project::ScriptSnapshot;
use crate::sync::record::scripts::{NamedScript, ScriptSink};
use std::collections::HashSet;

/// The real sink over the app's `ScriptStore`. `unsaved` holds the ids with unsaved editor text
/// or a pending draft, supplied by the Pine Studio.
pub struct StoreScriptSink<'a> {
    pub store: &'a mut ScriptStore,
    pub unsaved: HashSet<String>,
}

fn snapshot(script: &LocalScript) -> ScriptSnapshot {
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
}
