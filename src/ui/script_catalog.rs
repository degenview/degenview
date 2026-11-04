//! The saved scripts a chart can run, as the "+" menu and the chart settings list them.
//!
//! `ScriptStore::all_scripts` rescans the Scripts folder, so nothing may call it per frame: the
//! list is read at most every [`REFRESH`] and shared by every chart and the settings sheet, and a
//! caller asks for it only while a menu or the Scripts page is open.

use crate::pine_studio::model::{LocalScript, ScriptType};
use crate::pine_studio::store::ScriptStore;
use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(2);

/// Every saved indicator and strategy, by name. Libraries are not scripts a chart can run.
pub type Scripts = Arc<Vec<Arc<LocalScript>>>;

#[derive(Clone)]
struct Cache {
    scripts: Scripts,
    read_at: Instant,
}

fn cache_id() -> egui::Id {
    egui::Id::new("script-catalog")
}

/// The saved scripts, re-read from disk when the last read is older than [`REFRESH`].
pub fn scripts(ctx: &egui::Context) -> Scripts {
    let cached = ctx.data(|data| data.get_temp::<Cache>(cache_id()));
    if let Some(cache) = &cached
        && (cfg!(test) || cache.read_at.elapsed() < REFRESH)
    {
        return cache.scripts.clone();
    }
    let read = if cfg!(test) {
        None
    } else {
        ScriptStore::default_location().and_then(|mut store| store.all_scripts().ok())
    };
    match read {
        Some(all) => publish(ctx, all),
        // A failed read keeps the last list rather than emptying the menu.
        None => cached.map_or_else(|| publish(ctx, Vec::new()), |cache| cache.scripts),
    }
}

/// Makes `all` the list [`scripts`] returns until the next read. Libraries are dropped and the
/// rest sorted by lowercase name.
pub fn publish(ctx: &egui::Context, all: Vec<LocalScript>) -> Scripts {
    let mut scripts: Vec<Arc<LocalScript>> = all
        .into_iter()
        .filter(|script| script.script_type != ScriptType::Library)
        .map(Arc::new)
        .collect();
    scripts.sort_by_cached_key(|script| script.name.to_lowercase());
    let scripts = Arc::new(scripts);
    ctx.data_mut(|data| {
        data.insert_temp(
            cache_id(),
            Cache {
                scripts: scripts.clone(),
                read_at: Instant::now(),
            },
        );
    });
    scripts
}

/// The scripts whose name contains `query` (ignoring case and surrounding spaces): names that
/// start with it first, then the rest, each in the order given.
pub fn matching<'a>(
    scripts: impl IntoIterator<Item = &'a Arc<LocalScript>>,
    query: &str,
) -> Vec<&'a Arc<LocalScript>> {
    let needle = query.trim().to_lowercase();
    let (mut prefix, mut inside) = (Vec::new(), Vec::new());
    for script in scripts {
        let name = script.name.to_lowercase();
        if needle.is_empty() || name.starts_with(&needle) {
            prefix.push(script);
        } else if name.contains(&needle) {
            inside.push(script);
        }
    }
    prefix.extend(inside);
    prefix
}

#[cfg(test)]
pub(crate) fn sample(id: &str, name: &str, script_type: ScriptType) -> LocalScript {
    LocalScript {
        id: id.to_owned(),
        name: name.to_owned(),
        script_type,
        source: format!("//@version=6\nindicator(\"{name}\")\nplot(close)\n"),
        latest_revision_id: None,
        created_at_ms: 0,
        modified_at_ms: 0,
        is_favorite: false,
        compile_record: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(found: &[&'a Arc<LocalScript>]) -> Vec<&'a str> {
        found.iter().map(|script| script.name.as_str()).collect()
    }

    #[test]
    fn libraries_are_left_out_and_names_sort_ignoring_case() {
        let ctx = egui::Context::default();
        let list = publish(
            &ctx,
            vec![
                sample("1", "zigzag", ScriptType::Indicator),
                sample("2", "Alpha", ScriptType::Strategy),
                sample("3", "helpers", ScriptType::Library),
                sample("4", "beta", ScriptType::Indicator),
            ],
        );
        assert_eq!(
            names(&list.iter().collect::<Vec<_>>()),
            ["Alpha", "beta", "zigzag"]
        );
        assert_eq!(scripts(&ctx), list, "tests read only what they published");
    }

    #[test]
    fn a_match_is_a_case_blind_substring_with_prefixes_first() {
        let ctx = egui::Context::default();
        let list = publish(
            &ctx,
            vec![
                sample("1", "Bollinger Squeeze", ScriptType::Indicator),
                sample("2", "Squeeze Momentum", ScriptType::Indicator),
                sample("3", "RSI", ScriptType::Indicator),
            ],
        );
        assert_eq!(
            names(&matching(list.iter(), "  squeeze ")),
            ["Squeeze Momentum", "Bollinger Squeeze"]
        );
        assert_eq!(names(&matching(list.iter(), "")).len(), 3);
        assert!(matching(list.iter(), "macd").is_empty());
    }
}
