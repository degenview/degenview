//! Per-entity conflict resolution. Pure functions over JSON values: given the last synced
//! version (when one is known) and the local and remote versions, produce what both Macs should
//! end up with. Every rule is symmetric — `merge(local, remote)` and `merge(remote, local)` give
//! the same result — so two Macs resolving the same conflict independently converge instead of
//! trading edits forever.
//!
//! Policy summary:
//! * An edit on one side only is taken as is.
//! * An edit beats a delete.
//! * Watchlists merge entry by entry (a market is identified by provider and symbol).
//! * Object-valued settings merge key by key; other settings are last-writer-wins.
//! * Layouts and scripts never lose a version: the loser is kept as a conflict copy.
//! * Everything else is last-writer-wins by `(revision, modified time, device id)`.

use crate::sync::record::policy::EntityKind;
use crate::sync::record::project::canonical_json;
use crate::sync::record::sha256_hex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Orders two versions of one entity. Revisions count accepted edits on one Mac, so a tie is
/// common and falls through to the edit time and finally the device id, which is unique.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp {
    pub revision: u64,
    pub modified_ms: i64,
    pub device: String,
}

impl Stamp {
    pub fn new(revision: u64, modified_ms: i64, device: impl Into<String>) -> Self {
        Self {
            revision,
            modified_ms,
            device: device.into(),
        }
    }

    pub fn beats(&self, other: &Stamp) -> bool {
        self > other
    }
}

pub struct Side<'a> {
    pub value: &'a Value,
    pub stamp: Stamp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Both Macs should hold this.
    Resolved(Value),
    /// Both versions matter and neither may be lost: keep `winner` under the original identity
    /// and save `loser` as a separate, recoverable copy.
    Conflict { winner: Value, loser: Value },
}

pub fn merge(kind: EntityKind, base: Option<&Value>, local: Side, remote: Side) -> Outcome {
    if local.value == remote.value {
        return Outcome::Resolved(local.value.clone());
    }
    // Only one side moved away from the version they both knew.
    if base == Some(local.value) {
        return Outcome::Resolved(remote.value.clone());
    }
    if base == Some(remote.value) {
        return Outcome::Resolved(local.value.clone());
    }
    match kind {
        EntityKind::Watchlist => Outcome::Resolved(merge_watchlist(base, &local, &remote)),
        EntityKind::Setting => match (local.value, remote.value) {
            (Value::Object(l), Value::Object(r)) => {
                Outcome::Resolved(Value::Object(merge_objects(
                    base.and_then(Value::as_object),
                    l,
                    r,
                    &local.stamp,
                    &remote.stamp,
                )))
            }
            _ => Outcome::Resolved(lww(&local, &remote).clone()),
        },
        EntityKind::Layout => {
            if layout_structure_hash(local.value) == layout_structure_hash(remote.value) {
                // Only the name (or other cosmetic fields) differ: nothing structural is at risk.
                Outcome::Resolved(lww(&local, &remote).clone())
            } else {
                let (winner, loser) = order(&local, &remote);
                Outcome::Conflict {
                    winner: winner.clone(),
                    loser: loser.clone(),
                }
            }
        }
        EntityKind::Script => {
            let (winner, loser) = order(&local, &remote);
            if script_source(winner) == script_source(loser) {
                Outcome::Resolved(winner.clone())
            } else {
                Outcome::Conflict {
                    winner: winner.clone(),
                    loser: loser.clone(),
                }
            }
        }
        // A paper account is a live state machine, so two Macs that both moved it cannot be
        // merged field by field; the later edit keeps the account and the other survives whole.
        EntityKind::PaperAccount => {
            let (winner, loser) = order(&local, &remote);
            Outcome::Conflict {
                winner: winner.clone(),
                loser: loser.clone(),
            }
        }
        EntityKind::LayoutDrawing
        | EntityKind::DrawingAsset
        | EntityKind::ChartTemplate
        | EntityKind::DrawingTemplate
        | EntityKind::EditorTheme
        | EntityKind::Alert
        | EntityKind::Portfolio
        | EntityKind::PortfolioTransaction => Outcome::Resolved(lww(&local, &remote).clone()),
    }
}

fn lww<'a>(local: &Side<'a>, remote: &Side<'a>) -> &'a Value {
    if local.stamp.beats(&remote.stamp) {
        local.value
    } else {
        remote.value
    }
}

fn order<'a>(local: &Side<'a>, remote: &Side<'a>) -> (&'a Value, &'a Value) {
    if local.stamp.beats(&remote.stamp) {
        (local.value, remote.value)
    } else {
        (remote.value, local.value)
    }
}

// MARK: Three-way helpers

/// The three-way choice for one value: an unchanged side yields to the changed one, and two
/// different changes go to the later edit.
fn choose3(base: Option<&Value>, l: &Value, r: &Value, ls: &Stamp, rs: &Stamp) -> Value {
    if l == r {
        l.clone()
    } else if base == Some(l) {
        r.clone()
    } else if base == Some(r) || ls.beats(rs) {
        l.clone()
    } else {
        r.clone()
    }
}

/// Keeps a key present on one side only unless the other side deleted it unchanged.
fn survives(base: Option<&Value>, only_side_value: &Value) -> bool {
    base.is_none_or(|b| b != only_side_value)
}

fn merge_objects(
    base: Option<&Map<String, Value>>,
    local: &Map<String, Value>,
    remote: &Map<String, Value>,
    ls: &Stamp,
    rs: &Stamp,
) -> Map<String, Value> {
    let mut keys: Vec<&String> = local.keys().chain(remote.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut merged = Map::new();
    for key in keys {
        let b = base.and_then(|b| b.get(key));
        match (local.get(key), remote.get(key)) {
            (Some(l), Some(r)) => {
                // Nested objects merge the same way; anything else is a value choice.
                let value = match (l, r) {
                    (Value::Object(lo), Value::Object(ro)) if l != r => {
                        Value::Object(merge_objects(b.and_then(Value::as_object), lo, ro, ls, rs))
                    }
                    _ => choose3(b, l, r, ls, rs),
                };
                merged.insert(key.clone(), value);
            }
            (Some(only), None) | (None, Some(only)) if survives(b, only) => {
                merged.insert(key.clone(), only.clone());
            }
            _ => {}
        }
    }
    merged
}

// MARK: Watchlists

struct Entry {
    key: String,
    value: Value,
}

/// A market is identified by `InstrumentId::key`, so adding the same market on two Macs yields
/// one row however each spelled it (`BTC`, `btcusdt`). A section has no natural identity beyond
/// its id.
fn entry_key(entry: &Value) -> Option<String> {
    let object = entry.as_object()?;
    let (variant, body) = object.iter().next()?;
    match variant.as_str() {
        "Instrument" => {
            let instrument = body.get("instrument")?;
            // A source this build does not know keeps its raw spelling rather than going
            // unmerged (and being dropped).
            match serde_json::from_value::<crate::model::InstrumentId>(instrument.clone()) {
                Ok(parsed) => Some(format!("i:{}", parsed.key())),
                Err(_) => Some(format!(
                    "i:{}:{}",
                    instrument.get("source")?,
                    instrument.get("symbol")?.as_str()?
                )),
            }
        }
        "Section" => Some(format!("s:{}", body.get("id")?)),
        _ => None,
    }
}

fn entries_of(list: &Value) -> Vec<Entry> {
    let mut seen = HashSet::new();
    list.get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let key = entry_key(value)?;
            seen.insert(key.clone()).then(|| Entry {
                key,
                value: value.clone(),
            })
        })
        .collect()
}

fn merge_watchlist(base: Option<&Value>, local: &Side, remote: &Side) -> Value {
    let l_entries = entries_of(local.value);
    let r_entries = entries_of(remote.value);
    let b_entries = base.map(entries_of);
    let merged = merge_entries(
        b_entries.as_deref(),
        &l_entries,
        &r_entries,
        &local.stamp,
        &remote.stamp,
    );

    let (Some(l), Some(r)) = (local.value.as_object(), remote.value.as_object()) else {
        return lww(local, remote).clone();
    };
    let b = base.and_then(Value::as_object);
    let mut fields = merge_objects(b, l, r, &local.stamp, &remote.stamp);
    fields.insert("entries".into(), Value::Array(merged));
    // The list keeps this Mac's own row id (it differs only for each Mac's Favorites list).
    if let Some(id) = l.get("id") {
        fields.insert("id".into(), id.clone());
    }
    if let (Some(a), Some(b)) = (
        l.get("updated_at_ms").and_then(Value::as_i64),
        r.get("updated_at_ms").and_then(Value::as_i64),
    ) {
        fields.insert("updated_at_ms".into(), a.max(b).into());
    }
    let created = |m: &Map<String, Value>| m.get("created_at_ms").and_then(Value::as_i64);
    if let (Some(a), Some(b)) = (created(l), created(r)) {
        let earliest = [a, b].into_iter().filter(|t| *t > 0).min().unwrap_or(0);
        fields.insert("created_at_ms".into(), earliest.into());
    }
    Value::Object(fields)
}

fn merge_entries(
    base: Option<&[Entry]>,
    local: &[Entry],
    remote: &[Entry],
    ls: &Stamp,
    rs: &Stamp,
) -> Vec<Value> {
    let by_key = |entries: &[Entry]| -> HashMap<String, Value> {
        entries
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect()
    };
    let (l_map, r_map) = (by_key(local), by_key(remote));
    let b_map = base.map(by_key);
    let b_get = |key: &str| b_map.as_ref().and_then(|m| m.get(key));

    // Which entries survive, and with which content.
    let mut result: HashMap<String, Value> = HashMap::new();
    for key in l_map.keys().chain(r_map.keys()) {
        if result.contains_key(key) {
            continue;
        }
        match (l_map.get(key), r_map.get(key)) {
            (Some(l), Some(r)) => {
                result.insert(key.clone(), choose3(b_get(key), l, r, ls, rs));
            }
            (Some(only), None) | (None, Some(only)) if survives(b_get(key), only) => {
                result.insert(key.clone(), only.clone());
            }
            _ => {}
        }
    }

    // The order of entries both sides still have: whichever side reordered wins; two different
    // reorders go to the later edit.
    let common = |entries: &[Entry]| -> Vec<String> {
        entries
            .iter()
            .filter(|e| l_map.contains_key(&e.key) && r_map.contains_key(&e.key))
            .filter(|e| result.contains_key(&e.key))
            .map(|e| e.key.clone())
            .collect()
    };
    let (order_l, order_r) = (common(local), common(remote));
    let order_b = base.map(common);
    let skeleton = if order_l == order_r {
        order_l
    } else if order_b.as_ref() == Some(&order_l) {
        order_r
    } else if order_b.as_ref() == Some(&order_r) || ls.beats(rs) {
        order_l
    } else {
        order_r
    };
    let in_skeleton: HashSet<&String> = skeleton.iter().collect();

    // Entries only one side has are placed after the entry that precedes them on that side. Two
    // sides inserting at the same spot are ordered by key, not by who is "local".
    let mut runs: BTreeMap<Option<String>, Vec<Vec<String>>> = BTreeMap::new();
    for (entries, other) in [(local, &r_map), (remote, &l_map)] {
        let mut anchor: Option<String> = None;
        let mut open: Option<Vec<String>> = None;
        let flush = |open: &mut Option<Vec<String>>,
                     anchor: &Option<String>,
                     runs: &mut BTreeMap<Option<String>, Vec<Vec<String>>>| {
            if let Some(run) = open.take() {
                runs.entry(anchor.clone()).or_default().push(run);
            }
        };
        for entry in entries {
            if in_skeleton.contains(&entry.key) {
                flush(&mut open, &anchor, &mut runs);
                anchor = Some(entry.key.clone());
            } else if result.contains_key(&entry.key) && !other.contains_key(&entry.key) {
                open.get_or_insert_with(Vec::new).push(entry.key.clone());
            }
        }
        flush(&mut open, &anchor, &mut runs);
    }
    for list in runs.values_mut() {
        list.sort();
    }

    let mut ordered: Vec<String> = Vec::new();
    let place = |anchor: &Option<String>, ordered: &mut Vec<String>| {
        for run in runs.get(anchor).into_iter().flatten() {
            ordered.extend(run.iter().cloned());
        }
    };
    place(&None, &mut ordered);
    for key in &skeleton {
        ordered.push(key.clone());
        place(&Some(key.clone()), &mut ordered);
    }
    ordered
        .into_iter()
        .filter_map(|key| result.get(&key).cloned())
        .collect()
}

// MARK: Layouts, scripts, drawings

/// A layout's content apart from its name and bookkeeping: what a conflict could actually lose.
pub fn layout_structure_hash(layout: &Value) -> String {
    let mut stripped = layout.clone();
    if let Some(object) = stripped.as_object_mut() {
        for field in ["id", "name", "created_at_ms"] {
            object.remove(field);
        }
    }
    sha256_hex(&canonical_json(&stripped))
}

fn script_source(script: &Value) -> Option<&str> {
    script.get("source").and_then(Value::as_str)
}

/// "Name (conflict from Mac 2026-10-20)". The recoverable copy of a version that lost.
pub fn conflict_copy_name(name: &str, device_name: &str, date: &str) -> String {
    let device = device_name.trim();
    let device = if device.is_empty() {
        "another Mac"
    } else {
        device
    };
    format!("{} (conflict from {device} {date})", name.trim())
}

/// Moves a drawing to a new id inside a full layout row. Used when two Macs created different
/// drawings with the same per-chart id: the local one is renumbered so both survive.
pub fn renumber_drawing(layout: &mut Value, chart_id: u64, old_id: u64, new_id: u64) -> bool {
    let Some(charts) = layout.get_mut("charts").and_then(Value::as_array_mut) else {
        return false;
    };
    for chart in charts {
        if chart.get("id").and_then(Value::as_u64) != Some(chart_id) {
            continue;
        }
        let Some(drawings) = chart.get_mut("drawings").and_then(Value::as_array_mut) else {
            return false;
        };
        for drawing in drawings {
            let Some(body) = drawing
                .as_object_mut()
                .and_then(|variants| variants.values_mut().next())
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            if body.get("id").and_then(Value::as_u64) == Some(old_id) {
                body.insert("id".into(), new_id.into());
                return true;
            }
        }
    }
    false
}

/// The next free drawing id in a chart, counting both sides' drawings.
pub fn next_drawing_id(layout: &Value, chart_id: u64, also_taken: &[u64]) -> u64 {
    let used = layout
        .get("charts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|chart| chart.get("id").and_then(Value::as_u64) == Some(chart_id))
        .filter_map(|chart| chart.get("drawings").and_then(Value::as_array))
        .flatten()
        .filter_map(|drawing| drawing.as_object()?.values().next()?.get("id")?.as_u64());
    used.chain(also_taken.iter().copied()).max().unwrap_or(0) + 1
}

#[cfg(test)]
mod tests;
