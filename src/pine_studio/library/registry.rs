//! Answers `import user/Library/version` from the library scripts in the Pine Studio, so the
//! compiler can run anywhere (editor, preview, chart) without touching the store. Port of
//! `PineLibraryRegistry.swift`.
//!
//! A path is matched on its library name alone: the user and version parts address a published
//! TradingView library, a local library has neither, so both are accepted and ignored. The name
//! is the script's file name, or failing that the title in its `library("…")` declaration.

use crate::model::pine::script::{LocalScript, ScriptType};
use crate::pine::compiler::LibraryResolver;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Default)]
struct Catalog {
    by_name: HashMap<String, String>,
    by_title: HashMap<String, String>,
}

static CATALOG: OnceLock<RwLock<Catalog>> = OnceLock::new();

fn catalog() -> &'static RwLock<Catalog> {
    CATALOG.get_or_init(Default::default)
}

/// The process-wide registry the editor, preview and charts compile against.
#[derive(Clone, Copy, Default)]
pub struct LibraryRegistry;

impl LibraryRegistry {
    /// Replaces the catalog with the library scripts among `scripts`.
    pub fn publish(scripts: &[LocalScript]) {
        let mut next = Catalog::default();
        for script in scripts
            .iter()
            .filter(|s| s.script_type == ScriptType::Library)
        {
            next.by_name
                .insert(script.name.clone(), script.source.clone());
            if let Some(title) = title_of(&script.source) {
                next.by_title
                    .entry(title)
                    .or_insert_with(|| script.source.clone());
            }
        }
        if let Ok(mut guard) = catalog().write() {
            *guard = next;
        }
    }
}

impl LibraryResolver for LibraryRegistry {
    fn source_for_library(&self, path: &str) -> Option<String> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() != 3 {
            return None;
        }
        let guard = catalog().read().ok()?;
        guard
            .by_name
            .get(parts[1])
            .or_else(|| guard.by_title.get(parts[1]))
            .cloned()
    }
}

/// The name a `library("Title")` declaration gives, which is what an import path uses.
pub fn title_of(source: &str) -> Option<String> {
    for line in source.lines() {
        let Some(rest) = line.trim_start_matches([' ', '\t']).strip_prefix("library") else {
            continue;
        };
        let Some(rest) = rest.trim_start_matches([' ', '\t']).strip_prefix('(') else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '\t']);
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let inner = &rest[1..];
        let end = inner.find(quote)?;
        if end > 0 {
            return Some(inner[..end].to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_library_title() {
        assert_eq!(
            title_of("//@version=6\nlibrary(\"Math Utils\")\n").as_deref(),
            Some("Math Utils")
        );
        assert_eq!(title_of("library( 'x' )").as_deref(), Some("x"));
        assert_eq!(title_of("indicator(\"x\")"), None);
    }
}
