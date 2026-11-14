use super::*;
use serde_json::json;

fn st(revision: u64, modified: i64, device: &str) -> Stamp {
    Stamp::new(revision, modified, device)
}

fn inst(id: u64, symbol: &str) -> Value {
    json!({"Instrument": {"id": id, "instrument": {"source": "Binance", "symbol": symbol},
        "name": symbol, "label": symbol}})
}

fn sec(id: u64, title: &str) -> Value {
    json!({"Section": {"id": id, "title": title, "is_collapsed": false}})
}

fn wl(name: &str, entries: Vec<Value>) -> Value {
    json!({"id": 1, "name": name, "entries": entries, "is_favorites": false,
        "display": {"columns": ["Last"], "sort": "manual", "shows_description": true},
        "created_at_ms": 5, "updated_at_ms": 10})
}

fn symbols(list: &Value) -> Vec<String> {
    entries_of(list).into_iter().map(|e| e.key).collect()
}

fn merged(base: Option<&Value>, l: &Value, r: &Value) -> Value {
    match merge(
        EntityKind::Watchlist,
        base,
        Side {
            value: l,
            stamp: st(2, 100, "a"),
        },
        Side {
            value: r,
            stamp: st(2, 200, "b"),
        },
    ) {
        Outcome::Resolved(value) => value,
        other => panic!("{other:?}"),
    }
}

fn merged_swapped(base: Option<&Value>, l: &Value, r: &Value) -> Value {
    match merge(
        EntityKind::Watchlist,
        base,
        Side {
            value: r,
            stamp: st(2, 200, "b"),
        },
        Side {
            value: l,
            stamp: st(2, 100, "a"),
        },
    ) {
        Outcome::Resolved(value) => value,
        other => panic!("{other:?}"),
    }
}

// MARK: Stamps and the shared prelude

#[test]
fn stamps_order_by_revision_then_time_then_device() {
    assert!(st(3, 0, "a").beats(&st(2, 999, "z")));
    assert!(st(2, 5, "a").beats(&st(2, 4, "z")));
    assert!(st(2, 5, "b").beats(&st(2, 5, "a")));
    assert!(!st(2, 5, "a").beats(&st(2, 5, "a")));
}

#[test]
fn identical_versions_resolve_to_themselves() {
    let v = json!({"a": 1});
    let out = merge(
        EntityKind::Alert,
        None,
        Side {
            value: &v,
            stamp: st(1, 1, "a"),
        },
        Side {
            value: &v,
            stamp: st(1, 1, "b"),
        },
    );
    assert_eq!(out, Outcome::Resolved(v));
}

#[test]
fn an_edit_on_one_side_only_is_taken() {
    let base = json!({"v": 1});
    let edited = json!({"v": 2});
    let a = merge(
        EntityKind::Alert,
        Some(&base),
        Side {
            value: &base,
            stamp: st(9, 9, "a"),
        },
        Side {
            value: &edited,
            stamp: st(1, 1, "b"),
        },
    );
    assert_eq!(a, Outcome::Resolved(edited.clone()));
    let b = merge(
        EntityKind::Alert,
        Some(&base),
        Side {
            value: &edited,
            stamp: st(1, 1, "a"),
        },
        Side {
            value: &base,
            stamp: st(9, 9, "b"),
        },
    );
    assert_eq!(b, Outcome::Resolved(edited));
}

#[test]
fn independent_entities_resolve_last_writer_wins_symmetrically() {
    let l = json!({"v": 1});
    let r = json!({"v": 2});
    for kind in [
        EntityKind::Alert,
        EntityKind::ChartTemplate,
        EntityKind::DrawingTemplate,
        EntityKind::LayoutDrawing,
    ] {
        let a = merge(
            kind,
            None,
            Side {
                value: &l,
                stamp: st(1, 1, "a"),
            },
            Side {
                value: &r,
                stamp: st(1, 2, "b"),
            },
        );
        let b = merge(
            kind,
            None,
            Side {
                value: &r,
                stamp: st(1, 2, "b"),
            },
            Side {
                value: &l,
                stamp: st(1, 1, "a"),
            },
        );
        assert_eq!(a, Outcome::Resolved(r.clone()));
        assert_eq!(a, b);
    }
}

// MARK: Watchlists

#[test]
fn two_macs_adding_different_markets_both_keep_them() {
    let base = wl("Tech", vec![inst(1, "AAA")]);
    let a = wl("Tech", vec![inst(1, "AAA"), inst(2, "BBB")]);
    let b = wl("Tech", vec![inst(1, "AAA"), inst(3, "CCC")]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(symbols(&out).len(), 3);
    assert_eq!(out, merged_swapped(Some(&base), &a, &b));
}

#[test]
fn the_same_market_added_on_both_macs_appears_once() {
    let base = wl("Tech", vec![]);
    let a = wl("Tech", vec![inst(11, "BTC")]);
    let b = wl("Tech", vec![inst(22, "BTC")]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(symbols(&out).len(), 1);
    assert_eq!(out, merged_swapped(Some(&base), &a, &b));
}

#[test]
fn the_same_market_spelled_differently_on_two_macs_appears_once() {
    let spelled = |id: u64, symbol: &str| {
        json!({"Instrument": {"id": id, "instrument": {"source": "binance", "symbol": symbol},
            "name": symbol, "label": symbol}})
    };
    let base = wl("Tech", vec![]);
    let a = wl("Tech", vec![spelled(11, "btc")]);
    let b = wl("Tech", vec![spelled(22, "BTCUSDT")]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(symbols(&out).len(), 1);
    assert_eq!(out, merged_swapped(Some(&base), &a, &b));
}

#[test]
fn a_removal_on_one_side_wins_if_the_other_did_not_touch_it() {
    let base = wl("T", vec![inst(1, "AAA"), inst(2, "BBB")]);
    let a = wl("T", vec![inst(1, "AAA")]);
    let b = wl("T", vec![inst(1, "AAA"), inst(2, "BBB")]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(symbols(&out).len(), 1);
    assert_eq!(out, merged_swapped(Some(&base), &a, &b));
}

#[test]
fn an_edit_beats_a_removal() {
    let base = wl("T", vec![inst(1, "AAA"), sec(9, "Old title")]);
    let removed = wl("T", vec![inst(1, "AAA")]);
    let edited = wl("T", vec![inst(1, "AAA"), sec(9, "New title")]);
    let out = merged(Some(&base), &removed, &edited);
    assert_eq!(symbols(&out).len(), 2);
    assert_eq!(out, merged_swapped(Some(&base), &removed, &edited));
}

#[test]
fn both_removing_the_same_market_removes_it() {
    let base = wl("T", vec![inst(1, "AAA"), inst(2, "BBB")]);
    let a = wl("T", vec![inst(2, "BBB")]);
    let b = wl("T", vec![inst(2, "BBB")]);
    assert_eq!(symbols(&merged(Some(&base), &a, &b)).len(), 1);
}

#[test]
fn a_reorder_on_one_side_is_kept_when_the_other_added_something() {
    let base = wl("T", vec![inst(1, "A"), inst(2, "B"), inst(3, "C")]);
    let reordered = wl("T", vec![inst(3, "C"), inst(1, "A"), inst(2, "B")]);
    let added = wl(
        "T",
        vec![inst(1, "A"), inst(2, "B"), inst(3, "C"), inst(4, "D")],
    );
    let out = merged(Some(&base), &reordered, &added);
    let keys = symbols(&out);
    assert_eq!(keys.len(), 4);
    assert!(keys[0].ends_with(":C"), "{keys:?}");
    assert!(keys.iter().position(|k| k.ends_with(":D")).unwrap() > 0);
    assert_eq!(out, merged_swapped(Some(&base), &reordered, &added));
}

#[test]
fn additions_land_next_to_their_neighbours() {
    let base = wl("T", vec![inst(1, "A"), inst(2, "B"), inst(3, "C")]);
    let a = wl(
        "T",
        vec![inst(1, "A"), inst(9, "X"), inst(2, "B"), inst(3, "C")],
    );
    let b = wl(
        "T",
        vec![inst(1, "A"), inst(2, "B"), inst(3, "C"), inst(8, "Y")],
    );
    let keys = symbols(&merged(Some(&base), &a, &b));
    let order: Vec<_> = keys
        .iter()
        .map(|k| k.rsplit(':').next().unwrap().to_string())
        .collect();
    assert_eq!(order, ["A", "X", "B", "C", "Y"]);
}

#[test]
fn two_additions_at_the_same_spot_order_deterministically() {
    let base = wl("T", vec![inst(1, "A")]);
    let a = wl("T", vec![inst(1, "A"), inst(2, "M")]);
    let b = wl("T", vec![inst(1, "A"), inst(3, "N")]);
    assert_eq!(
        merged(Some(&base), &a, &b),
        merged_swapped(Some(&base), &a, &b)
    );
}

#[test]
fn concurrent_renames_go_to_the_later_edit() {
    let base = wl("Old", vec![]);
    let a = wl("A's name", vec![]);
    let b = wl("B's name", vec![]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(out["name"], "B's name");
    assert_eq!(out, merged_swapped(Some(&base), &a, &b));
}

#[test]
fn a_rename_and_a_new_market_both_survive() {
    let base = wl("Old", vec![inst(1, "A")]);
    let a = wl("New name", vec![inst(1, "A")]);
    let b = wl("Old", vec![inst(1, "A"), inst(2, "B")]);
    let out = merged(Some(&base), &a, &b);
    assert_eq!(out["name"], "New name");
    assert_eq!(symbols(&out).len(), 2);
}

#[test]
fn display_settings_merge_field_by_field() {
    let mut base = wl("T", vec![]);
    let mut a = base.clone();
    let mut b = base.clone();
    a["display"]["sort"] = json!("alpha");
    b["display"]["shows_description"] = json!(false);
    base["display"]["sort"] = json!("manual");
    let out = merged(Some(&base), &a, &b);
    assert_eq!(out["display"]["sort"], "alpha");
    assert_eq!(out["display"]["shows_description"], false);
}

#[test]
fn the_merged_list_keeps_this_macs_own_id_and_the_widest_timestamps() {
    let mut a = wl("F", vec![inst(1, "A")]);
    let mut b = wl("F", vec![inst(2, "B")]);
    a["id"] = json!(111);
    b["id"] = json!(222);
    a["created_at_ms"] = json!(50);
    b["created_at_ms"] = json!(20);
    b["updated_at_ms"] = json!(99);
    let out = merged(None, &a, &b);
    assert_eq!(out["id"], 111);
    assert_eq!(out["created_at_ms"], 20);
    assert_eq!(out["updated_at_ms"], 99);
}

#[test]
fn without_a_known_base_nothing_is_deleted() {
    let a = wl("T", vec![inst(1, "A")]);
    let b = wl("T", vec![inst(2, "B")]);
    assert_eq!(symbols(&merged(None, &a, &b)).len(), 2);
}

#[test]
fn duplicate_market_rows_inside_one_list_collapse() {
    let a = wl("T", vec![inst(1, "A"), inst(2, "A")]);
    assert_eq!(entries_of(&a).len(), 1);
}

#[test]
fn malformed_entries_are_ignored_not_fatal() {
    let a = json!({"id": 1, "name": "x", "entries": [1, "x", {"Weird": {}}, {"Instrument": {}}, inst(1, "A")]});
    assert_eq!(entries_of(&a).len(), 1);
    let b = json!({"id": 1, "name": "x"});
    assert!(entries_of(&b).is_empty());
}

/// Deterministic pseudo-random edits: whatever two Macs do, they converge and keep every
/// market that was added.
#[test]
fn random_concurrent_edits_converge_and_keep_additions() {
    let mut seed: u64 = 0x5eed;
    let mut next = move |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for round in 0..300 {
        let mut id = 100;
        let mut base_entries = Vec::new();
        for k in 0..next(6) {
            id += 1;
            base_entries.push(inst(id, &format!("B{k}")));
        }
        let base = wl("Base", base_entries.clone());
        let mut edit = |mut entries: Vec<Value>, tag: &str| {
            for _ in 0..next(4) {
                match next(3) {
                    0 if !entries.is_empty() => {
                        let at = next(entries.len() as u64) as usize;
                        entries.remove(at);
                    }
                    1 => {
                        id += 1;
                        let at = next(entries.len() as u64 + 1) as usize;
                        entries.insert(at, inst(id, &format!("{tag}{id}")));
                    }
                    _ if entries.len() > 1 => {
                        let from = next(entries.len() as u64) as usize;
                        let item = entries.remove(from);
                        let to = next(entries.len() as u64 + 1) as usize;
                        entries.insert(to, item);
                    }
                    _ => {}
                }
            }
            entries
        };
        let a = wl("Base", edit(base_entries.clone(), "A"));
        let b = wl("Base", edit(base_entries.clone(), "B"));
        let out = merged(Some(&base), &a, &b);
        let swapped = merged_swapped(Some(&base), &a, &b);
        assert_eq!(out, swapped, "round {round}");
        // Idempotent: merging the result with either input again changes nothing.
        let again = merged(Some(&out), &out, &out);
        assert_eq!(again, out);
        // Every market added on either side is present, once.
        let result: HashSet<_> = symbols(&out).into_iter().collect();
        assert_eq!(
            result.len(),
            symbols(&out).len(),
            "duplicates in round {round}"
        );
        let base_keys: HashSet<_> = symbols(&base).into_iter().collect();
        for side in [&a, &b] {
            for key in symbols(side) {
                if !base_keys.contains(&key) {
                    assert!(result.contains(&key), "lost {key} in round {round}");
                }
            }
        }
    }
}

#[test]
fn three_macs_converge_in_any_exchange_order() {
    let base = wl("T", vec![inst(1, "A")]);
    let m1 = wl("T", vec![inst(1, "A"), inst(2, "B")]);
    let m2 = wl("T", vec![inst(3, "C"), inst(1, "A")]);
    let m3 = wl("T", vec![]);
    let pair = |l: &Value, r: &Value, ls: Stamp, rs: Stamp| match merge(
        EntityKind::Watchlist,
        Some(&base),
        Side {
            value: l,
            stamp: ls,
        },
        Side {
            value: r,
            stamp: rs,
        },
    ) {
        Outcome::Resolved(v) => v,
        other => panic!("{other:?}"),
    };
    let s = |d: &str, t: i64| st(2, t, d);
    let ab = pair(&m1, &m2, s("1", 1), s("2", 2));
    let abc = pair(&ab, &m3, s("2", 2), s("3", 3));
    let bc = pair(&m2, &m3, s("2", 2), s("3", 3));
    let a_bc = pair(&m1, &bc, s("1", 1), s("3", 3));
    assert_eq!(
        symbols(&abc).into_iter().collect::<HashSet<_>>(),
        symbols(&a_bc).into_iter().collect::<HashSet<_>>()
    );
}

// MARK: Settings

#[test]
fn object_settings_merge_per_key() {
    let base = json!({"trend": {"color": "red"}, "rect": {"color": "blue"}});
    let a = json!({"trend": {"color": "green"}, "rect": {"color": "blue"}});
    let b = json!({"trend": {"color": "red"}, "rect": {"color": "blue"}, "fib": {"color": "gold"}});
    let out = merge(
        EntityKind::Setting,
        Some(&base),
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(
        out,
        Outcome::Resolved(
            json!({"trend": {"color": "green"}, "rect": {"color": "blue"}, "fib": {"color": "gold"}})
        )
    );
}

#[test]
fn a_key_removed_on_one_side_stays_removed_unless_edited_on_the_other() {
    let base = json!({"a": 1, "b": 2});
    let removed = json!({"b": 2});
    let kept = json!({"a": 1, "b": 3});
    let out = merge(
        EntityKind::Setting,
        Some(&base),
        Side {
            value: &removed,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &kept,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(out, Outcome::Resolved(json!({"b": 3})));
    let edited = json!({"a": 9, "b": 2});
    let out = merge(
        EntityKind::Setting,
        Some(&base),
        Side {
            value: &removed,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &edited,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(out, Outcome::Resolved(json!({"a": 9, "b": 2})));
}

#[test]
fn scalar_settings_are_last_writer_wins() {
    let l = json!("light");
    let r = json!("dark");
    let a = merge(
        EntityKind::Setting,
        None,
        Side {
            value: &l,
            stamp: st(1, 5, "a"),
        },
        Side {
            value: &r,
            stamp: st(1, 9, "b"),
        },
    );
    let b = merge(
        EntityKind::Setting,
        None,
        Side {
            value: &r,
            stamp: st(1, 9, "b"),
        },
        Side {
            value: &l,
            stamp: st(1, 5, "a"),
        },
    );
    assert_eq!(a, Outcome::Resolved(r));
    assert_eq!(a, b);
}

// MARK: Layouts and scripts

fn layout(name: &str, symbol: &str) -> Value {
    json!({"id": 10, "name": name, "created_at_ms": 1, "charts": [{"id": 1, "symbol": symbol}]})
}

#[test]
fn a_layout_conflict_keeps_both_versions() {
    let a = layout("Main", "BTC");
    let b = layout("Main", "ETH");
    let out = merge(
        EntityKind::Layout,
        None,
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(
        out,
        Outcome::Conflict {
            winner: b.clone(),
            loser: a.clone()
        }
    );
    let swapped = merge(
        EntityKind::Layout,
        None,
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
    );
    assert_eq!(
        swapped,
        Outcome::Conflict {
            winner: b,
            loser: a
        }
    );
}

#[test]
fn a_layout_that_differs_only_by_name_is_not_a_conflict() {
    let a = layout("Main", "BTC");
    let b = layout("Renamed", "BTC");
    let out = merge(
        EntityKind::Layout,
        None,
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(out, Outcome::Resolved(b));
}

fn script(source: &str, name: &str) -> Value {
    json!({"id": "0123456789abcdef0123456789abcdef", "name": name, "source": source})
}

#[test]
fn two_edits_to_one_script_never_lose_either() {
    let a = script("plot(close)", "My Strategy");
    let b = script("plot(open)", "My Strategy");
    let out = merge(
        EntityKind::Script,
        None,
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(
        out,
        Outcome::Conflict {
            winner: b,
            loser: a
        }
    );
}

#[test]
fn a_script_renamed_on_one_mac_and_untouched_source_is_not_a_conflict() {
    let a = script("plot(close)", "Old");
    let b = script("plot(close)", "New");
    let out = merge(
        EntityKind::Script,
        None,
        Side {
            value: &a,
            stamp: st(2, 1, "a"),
        },
        Side {
            value: &b,
            stamp: st(2, 2, "b"),
        },
    );
    assert_eq!(out, Outcome::Resolved(b));
}

#[test]
fn conflict_copy_names_say_where_they_came_from() {
    assert_eq!(
        conflict_copy_name("My Strategy ", "Studio Mac", "2026-10-20"),
        "My Strategy (conflict from Studio Mac 2026-10-20)"
    );
    assert_eq!(
        conflict_copy_name("S", "  ", "d"),
        "S (conflict from another Mac d)"
    );
}

// MARK: Drawings

fn layout_with_drawings(ids: &[u64]) -> Value {
    let drawings: Vec<_> = ids
        .iter()
        .map(|id| json!({"TrendLine": {"id": id, "a": {"time_ms": 1, "price": 1.0}}}))
        .collect();
    json!({"id": 10, "charts": [{"id": 2, "drawings": drawings}, {"id": 3, "drawings": []}]})
}

#[test]
fn a_drawing_can_be_renumbered_inside_its_chart() {
    let mut layout = layout_with_drawings(&[1, 5]);
    assert!(renumber_drawing(&mut layout, 2, 5, 9));
    assert_eq!(layout["charts"][0]["drawings"][1]["TrendLine"]["id"], 9);
    assert!(!renumber_drawing(&mut layout, 2, 5, 9));
    assert!(!renumber_drawing(&mut layout, 3, 1, 9));
    assert!(!renumber_drawing(&mut layout, 77, 1, 9));
}

#[test]
fn the_next_drawing_id_avoids_both_sides() {
    let layout = layout_with_drawings(&[1, 5]);
    assert_eq!(next_drawing_id(&layout, 2, &[]), 6);
    assert_eq!(next_drawing_id(&layout, 2, &[40]), 41);
    assert_eq!(next_drawing_id(&layout, 3, &[]), 1);
}
