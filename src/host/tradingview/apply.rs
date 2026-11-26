//! Writes a plan into the stores: scripts first (layouts refer to them), then watchlists, then
//! layouts. One that cannot be written is reported and does not stop the rest.

use crate::clock::now_ms;
use crate::i18n::tr;
use crate::model::SavedView;
use crate::model::tradingview::plan::{Plan, Summary};
use crate::model::watchlist::new_id;
use crate::store::SharedStore;
use crate::store::scripts::{ScriptStore, declared_type};
use crate::store::watchlist::WatchlistStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Where an import goes.
#[derive(Clone)]
pub struct Targets {
    pub store: SharedStore,
    pub watchlist: Arc<Mutex<WatchlistStore>>,
    pub scripts: Arc<dyn Fn() -> Option<ScriptStore> + Send + Sync>,
}

pub fn apply(plan: &Plan, targets: &Targets) -> Summary {
    let mut summary = Summary::default();
    let scripts = add_scripts(plan, targets, &mut summary);
    add_watchlists(plan, targets, &mut summary);
    add_layouts(plan, targets, &scripts, &mut summary);
    summary
}

fn failed(name: &str, error: impl std::fmt::Display) -> String {
    tr!("{name}: {error}", name = name, error = error)
}

/// Adds the scripts; returns each TradingView id's new local id.
fn add_scripts(plan: &Plan, targets: &Targets, summary: &mut Summary) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    if plan.scripts.is_empty() {
        return ids;
    }
    let Some(mut library) = (targets.scripts)() else {
        summary
            .failures
            .push(tr!("The script library couldn’t be opened.").to_owned());
        return ids;
    };
    for script in &plan.scripts {
        let kind = declared_type(&script.source);
        match library.create(&script.name, kind, Some(&script.source), true) {
            Ok(created) => {
                ids.insert(script.source_id.clone(), created.id);
                summary.scripts += 1;
            }
            Err(error) => summary.failures.push(failed(&script.name, error)),
        }
    }
    ids
}

fn add_watchlists(plan: &Plan, targets: &Targets, summary: &mut Summary) {
    if plan.watchlists.is_empty() {
        return;
    }
    let mut store = crate::lock::lock(&targets.watchlist);
    for planned in &plan.watchlists {
        match store.add_list(planned.list.clone()) {
            Ok(_) => summary.watchlists += 1,
            Err(error) => summary.failures.push(failed(&planned.list.name, error)),
        }
    }
}

fn add_layouts(
    plan: &Plan,
    targets: &Targets,
    scripts: &HashMap<String, String>,
    summary: &mut Summary,
) {
    let added: Vec<SavedView> = plan
        .layouts
        .iter()
        .filter(|layout| !layout.charts.is_empty())
        .map(|layout| {
            let mut charts = layout.charts.clone();
            // An applied script of the account's own now has a script in the library.
            for (chart_id, instance_id, script) in &layout.own_scripts {
                if let Some(instance) = charts
                    .iter_mut()
                    .filter(|chart| chart.id == *chart_id)
                    .flat_map(|chart| &mut chart.pine_instances)
                    .find(|instance| instance.id == *instance_id)
                {
                    instance.script_id = scripts.get(script).cloned();
                }
            }
            SavedView {
                id: new_id(),
                name: layout.name.clone(),
                timeframe: layout.timeframe,
                created_at_ms: now_ms(),
                charts,
                columns: Some(layout.columns.clone()),
                candle_count: 0,
                last_opened_at_ms: None,
                autosave: None,
                sync: Default::default(),
            }
        })
        .collect();
    if added.is_empty() {
        return;
    }
    let written = {
        let store = crate::lock::lock(&targets.store);
        let mut views = store.saved_views();
        views.extend(added.iter().cloned());
        store.replace_saved_views(&views)
    };
    match written {
        Ok(()) => {
            summary.layouts = added
                .iter()
                .map(|view| (view.id, view.name.clone()))
                .collect();
            summary.drawings = plan.drawing_count();
        }
        Err(error) => summary.failures.push(failed(tr!("Saved layouts"), error)),
    }
}
