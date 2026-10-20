//! A paper-trading account as one synced record.
//!
//! An account is a live state machine: its working orders fill from quotes, and each fill moves
//! cash, positions and history together. Syncing those rows one by one would let two Macs fill the
//! same order and disagree about the result, so everything an account owns travels as a single
//! bundle, and one Mac at a time (the `owner`) runs it. The others show a mirror.

use super::ids::{Direction, IdSpace, map_fields_with};
use super::project::{Projected, project_bundle};
use serde_json::{Map, Value};

/// The bundle key of each account-scoped collection, and the table it is stored in.
pub const COLLECTIONS: [(&str, &str); 6] = [
    ("orders", "paper_order"),
    ("fills", "paper_fill"),
    ("positions", "paper_position"),
    ("order_events", "paper_order_event"),
    ("closed_trades", "paper_closed_trade"),
    ("journal", "paper_journal"),
];

/// The fields of an order that hold ids; an order also appears inside each of its events.
const ORDER_IDS: [&str; 4] = ["id", "account_id", "parent_order_id", "oco_group_id"];

fn id_fields(collection: &str) -> &'static [&'static str] {
    match collection {
        "orders" => &ORDER_IDS,
        "fills" => &["id", "order_id", "account_id"],
        "positions" => &["account_id"],
        "order_events" => &["id", "order_id", "account_id"],
        _ => &["id", "account_id"],
    }
}

/// What one account owns, in this database's own ids.
#[derive(Debug, Clone, PartialEq)]
pub struct Parts {
    pub account: Value,
    /// The device running the account.
    pub owner: String,
    /// One list per entry of [`COLLECTIONS`], in that order.
    pub rows: Vec<Vec<Value>>,
}

impl Parts {
    pub fn empty(account: Value, owner: String) -> Self {
        Self {
            account,
            owner,
            rows: vec![Vec::new(); COLLECTIONS.len()],
        }
    }

    pub fn account_id(&self) -> Option<u64> {
        self.account.get("id").and_then(Value::as_u64)
    }
}

/// The bundle as a JSON object, ids unchanged.
pub fn bundle_value(parts: &Parts) -> Value {
    let mut bundle = Map::new();
    bundle.insert("account".into(), parts.account.clone());
    bundle.insert("owner".into(), Value::String(parts.owner.clone()));
    for ((key, _), rows) in COLLECTIONS.iter().zip(&parts.rows) {
        bundle.insert((*key).into(), Value::Array(rows.clone()));
    }
    Value::Object(bundle)
}

/// Splits a bundle into its parts, ids unchanged. `None` when the shape is not a bundle.
pub fn split_bundle(bundle: &Value) -> Option<Parts> {
    let object = bundle.as_object()?;
    let mut rows = Vec::new();
    for (key, _) in COLLECTIONS {
        rows.push(object.get(key)?.as_array()?.clone());
    }
    Some(Parts {
        account: object.get("account")?.clone(),
        owner: object.get("owner")?.as_str()?.to_string(),
        rows,
    })
}

/// The owner of an account that nobody runs: a conflict copy waits for the user to take it over.
pub const PAUSED: &str = "paused";

/// Rewrites every id in the parts through `map`, in place.
pub fn map_parts_with(parts: &mut Parts, map: &dyn Fn(u64) -> u64) {
    map_fields_with(&mut parts.account, &["id"], map);
    for ((key, _), rows) in COLLECTIONS.iter().zip(parts.rows.iter_mut()) {
        for row in rows {
            map_fields_with(row, id_fields(key), map);
            if *key == "order_events"
                && let Some(order) = row.get_mut("order")
            {
                map_fields_with(order, &ORDER_IDS, map);
            }
        }
    }
}

/// Rewrites every id in the parts between this database's ids and the shared ones.
pub fn map_parts(parts: &mut Parts, ids: IdSpace, direction: Direction) {
    map_parts_with(parts, &|id| ids.map(direction, id));
}

/// Gives every record in the parts an id of its own, as a copy of the account needs: the order
/// ids, fill ids and the rest all change together, so the copy refers only to itself.
pub fn renumber(parts: &mut Parts, mut fresh: impl FnMut() -> u64) {
    let mut own: Vec<u64> = parts.account_id().into_iter().collect();
    for ((key, _), rows) in COLLECTIONS.iter().zip(&parts.rows) {
        for row in rows {
            if *key != "positions" {
                own.extend(row.get("id").and_then(Value::as_u64));
            }
            if *key == "orders" {
                own.extend(row.get("oco_group_id").and_then(Value::as_u64));
            }
        }
    }
    let mapping: std::collections::HashMap<u64, u64> =
        own.into_iter().map(|id| (id, fresh())).collect();
    map_parts_with(parts, &|id| mapping.get(&id).copied().unwrap_or(id));
}

/// The entity an account syncs as: the bundle with shared ids, named by the shared account id.
pub fn project(ids: IdSpace, mut parts: Parts) -> Option<Projected> {
    let local = parts.account_id()?;
    map_parts(&mut parts, ids, Direction::ToGlobal);
    Some(project_bundle(
        ids.to_global(local).to_string(),
        local.to_string(),
        &bundle_value(&parts),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Parts {
        let order = json!({"id": 5, "account_id": 2, "parent_order_id": 4, "oco_group_id": null});
        Parts {
            account: json!({"id": 2, "name": "Paper"}),
            owner: "mac-a".into(),
            rows: vec![
                vec![order.clone()],
                vec![json!({"id": 6, "order_id": 5, "account_id": 2})],
                vec![json!({"account_id": 2, "signed_quantity": 1.0})],
                vec![json!({"id": 7, "order_id": 5, "account_id": 2, "order": order})],
                vec![json!({"id": 8, "account_id": 2})],
                vec![json!({"id": 9, "account_id": 2, "message": "x"})],
            ],
        }
    }

    #[test]
    fn every_id_is_shared_and_comes_back_unchanged() {
        let ids = IdSpace::new(0x1234);
        let original = sample();
        let mut mapped = original.clone();
        map_parts(&mut mapped, ids, Direction::ToGlobal);
        let global = |n| ids.to_global(n);
        assert_eq!(mapped.account["id"], global(2));
        assert_eq!(mapped.rows[0][0]["id"], global(5));
        assert_eq!(mapped.rows[0][0]["parent_order_id"], global(4));
        assert_eq!(mapped.rows[0][0]["oco_group_id"], Value::Null);
        assert_eq!(mapped.rows[1][0]["order_id"], global(5));
        assert_eq!(mapped.rows[2][0]["account_id"], global(2));
        assert_eq!(mapped.rows[3][0]["order"]["id"], global(5));
        assert_eq!(mapped.rows[3][0]["order"]["parent_order_id"], global(4));
        assert_eq!(mapped.rows[5][0]["account_id"], global(2));
        // No small id is left anywhere in the shared form.
        let text = bundle_value(&mapped).to_string();
        for small in ["\"id\":2,", "\"order_id\":5", "\"account_id\":2"] {
            assert!(!text.contains(small), "{small} in {text}");
        }
        map_parts(&mut mapped, ids, Direction::ToLocal);
        assert_eq!(mapped, original);
    }

    #[test]
    fn a_copy_refers_only_to_itself() {
        let mut parts = sample();
        let mut next = 1_000;
        renumber(&mut parts, || {
            next += 1;
            next
        });
        let text = bundle_value(&parts).to_string();
        // None of the original small ids survive, and the links between records still hold.
        for old in ["\"id\":2", "\"id\":5", "\"order_id\":5", "\"account_id\":2"] {
            assert!(!text.contains(old), "{old} in {text}");
        }
        let order = parts.rows[0][0]["id"].clone();
        assert_eq!(parts.rows[1][0]["order_id"], order);
        assert_eq!(parts.rows[3][0]["order"]["id"], order);
        let account = parts.account["id"].clone();
        assert!(
            parts
                .rows
                .iter()
                .flatten()
                .all(|row| row["account_id"] == account)
        );
        let ids: std::collections::HashSet<_> = [
            parts.account["id"].as_u64().unwrap(),
            parts.rows[0][0]["id"].as_u64().unwrap(),
            parts.rows[1][0]["id"].as_u64().unwrap(),
            parts.rows[3][0]["id"].as_u64().unwrap(),
            parts.rows[4][0]["id"].as_u64().unwrap(),
            parts.rows[5][0]["id"].as_u64().unwrap(),
        ]
        .into();
        assert_eq!(ids.len(), 6, "every record got a different id");
    }

    #[test]
    fn a_bundle_splits_back_into_its_parts() {
        let parts = sample();
        assert_eq!(split_bundle(&bundle_value(&parts)), Some(parts));
        assert_eq!(split_bundle(&json!({"account": {}})), None);
    }

    #[test]
    fn two_databases_name_the_same_small_account_differently() {
        let (a, b) = (IdSpace::new(1), IdSpace::new(2));
        let one = project(a, sample()).unwrap();
        let other = project(b, sample()).unwrap();
        assert_ne!(one.sync_id, other.sync_id);
        assert_eq!(one.local_id, "2");
        assert_ne!(one.hash, other.hash);
    }

    #[test]
    fn the_projection_does_not_depend_on_row_key_order() {
        let ids = IdSpace::new(9);
        let a = project(ids, sample()).unwrap();
        let mut shuffled = sample();
        shuffled.account = json!({"name": "Paper", "id": 2});
        assert_eq!(project(ids, shuffled).unwrap().hash, a.hash);
    }
}
