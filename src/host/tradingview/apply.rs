//! The import's work on the stores, all of it run off the frame: reading what an import must not
//! take over, and writing a plan (scripts first, since layouts refer to them, then watchlists,
//! then layouts). One that cannot be written is reported and does not stop the rest.

use crate::clock::now_ms;
use crate::i18n::tr;
use crate::model::tradingview::Library;
use crate::model::tradingview::plan::{
    self, Mappings, Placements, Plan, Selection, Summary, Taken,
};
use crate::model::watchlist::new_id;
use crate::model::{ChartAppearance, SavedView};
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

/// What an import is written from.
pub struct Order {
    pub library: Library,
    pub selection: Selection,
    pub mappings: Mappings,
    pub placements: Placements,
}

/// The names an import must not take over, and the look a new chart gets.
pub fn surroundings(targets: &Targets) -> (Taken, ChartAppearance) {
    let (layouts, base) = {
        let store = crate::lock::lock(&targets.store);
        (
            store
                .saved_views()
                .into_iter()
                .map(|view| view.name)
                .collect(),
            store.default_chart_template().look.unwrap_or_default(),
        )
    };
    let watchlists = crate::lock::lock(&targets.watchlist)
        .lists()
        .iter()
        .map(|list| list.name.clone())
        .collect();
    let scripts = (targets.scripts)()
        .map(|library| library.names_in_use().into_iter().collect())
        .unwrap_or_default();
    (
        Taken {
            layouts,
            watchlists,
            scripts,
        },
        base,
    )
}

/// Plans `order` against the names in use now, and writes it.
pub fn commit(order: &Order, targets: &Targets) -> Summary {
    let (taken, base) = surroundings(targets);
    let plan = plan::plan(
        &order.library,
        &order.selection,
        &order.mappings,
        &order.placements,
        &taken,
        &base,
    );
    apply(&plan, targets)
}

pub(super) fn apply(plan: &Plan, targets: &Targets) -> Summary {
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
                summary.scripts.push(created.name);
            }
            Err(error) => summary.failures.push(failed(&script.name, error)),
        }
    }
    ids
}

fn add_watchlists(plan: &Plan, targets: &Targets, summary: &mut Summary) {
    if plan.watchlists.iter().all(|list| list.markets == 0) {
        return;
    }
    let planned = || plan.watchlists.iter().filter(|list| list.markets > 0);
    let lists = planned().map(|planned| planned.list.clone()).collect();
    let added = crate::lock::lock(&targets.watchlist).add_lists(lists);
    for (planned, result) in planned().zip(added) {
        match result {
            Ok(_) => summary.watchlists.push(planned.list.name.clone()),
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
