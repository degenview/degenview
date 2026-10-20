//! Cost of sync on the paths users feel. Ignored by default (timings depend on the machine):
//! `cargo test --release --lib sync::perf_tests -- --ignored --nocapture`.

use super::apply::{ApplyCtx, apply_batch};
use super::core::{RemoteBatch, RemoteRecord};
use super::record::{RecordName, SyncRecord};
use super::scripts::NoScripts;
use crate::store::Store;
use crate::store::sync::RawDoc;
use serde_json::{Value, json};
use std::time::Instant;

fn store(enabled: bool) -> Store {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    state.enabled = enabled;
    store.save_sync_state(&state).unwrap();
    store
}

fn timed<R>(label: &str, runs: u32, mut f: impl FnMut() -> R) -> f64 {
    f();
    let start = Instant::now();
    for _ in 0..runs {
        f();
    }
    let per = start.elapsed().as_secs_f64() * 1_000.0 / f64::from(runs);
    println!("{label:<62} {per:>9.3} ms");
    per
}

fn layout(id: u64, drawings: usize) -> RawDoc {
    let list: Vec<Value> = (0..drawings)
        .map(|n| json!({"TrendLine": {"id": n, "a": {"time_ms": 1, "price": 1.5}, "b": {"time_ms": 2, "price": 2.5}}}))
        .collect();
    RawDoc(json!({"id": id, "name": "Layout", "charts": [
        {"id": 1, "symbol": "BTCUSDT", "drawings": list},
        {"id": 2, "symbol": "ETHUSDT", "drawings": []}], "last_opened_at_ms": 5, "autosave": true}))
}

fn watchlist(id: u64, entries: usize) -> RawDoc {
    let rows: Vec<Value> = (0..entries)
        .map(|n| json!({"Instrument": {"id": n, "instrument": {"source": "binance", "symbol": format!("S{n}")}, "name": "n", "label": "l"}}))
        .collect();
    RawDoc(json!({"id": id, "name": "List", "entries": rows, "is_favorites": false}))
}

fn alert(id: u64, tick: i64) -> RawDoc {
    RawDoc(
        json!({"id": id, "asset": {"symbol": "BTC"}, "condition": {"Above": 100.0}, "currency": "USD",
        "frequency": "Once", "state": "Active", "note": "", "armed": tick % 2 == 0,
        "previous_value": tick as f64, "last_fingerprint": format!("f{tick}"),
        "created_at_ms": 1, "updated_at_ms": tick, "webhook_endpoint_ids": [], "webhook_message": null}),
    )
}

#[test]
#[ignore]
fn what_sync_costs() {
    println!("\n--- write overhead (sync off -> on) ---");
    let layouts: Vec<_> = (1..=30).map(|id| layout(id, 20)).collect();
    let off = store(false);
    let on = store(true);
    let l_off = timed("30 layouts x 20 drawings, sync OFF", 50, || {
        off.replace_documents("saved_view", &layouts).unwrap()
    });
    let l_on = timed(
        "30 layouts x 20 drawings, sync ON (unchanged rewrite)",
        50,
        || on.replace_documents("saved_view", &layouts).unwrap(),
    );
    let mut edited = layouts.clone();
    let mut n = 0;
    let l_edit = timed(
        "30 layouts x 20 drawings, sync ON (one drawing edited)",
        50,
        || {
            n += 1;
            edited[3] = layout(4, 20);
            edited[3].0["charts"][0]["drawings"][0]["TrendLine"]["a"]["price"] =
                json!(f64::from(n));
            on.replace_documents("saved_view", &edited).unwrap()
        },
    );

    let lists: Vec<_> = (1..=20).map(|id| watchlist(id, 50)).collect();
    let w_off = timed("20 watchlists x 50 entries, sync OFF", 50, || {
        off.replace_documents("watchlist", &lists).unwrap()
    });
    let w_on = timed(
        "20 watchlists x 50 entries, sync ON (unchanged)",
        50,
        || on.replace_documents("watchlist", &lists).unwrap(),
    );

    let mut tick = 0;
    let alerts = |tick: i64| (1..=100).map(|id| alert(id, tick)).collect::<Vec<_>>();
    let a_off = timed("100 alerts heartbeat rewrite, sync OFF", 100, || {
        tick += 1;
        off.replace_documents("price_alert", &alerts(tick)).unwrap()
    });
    let a_on = timed(
        "100 alerts heartbeat rewrite, sync ON (queues nothing)",
        100,
        || {
            tick += 1;
            on.replace_documents("price_alert", &alerts(tick)).unwrap()
        },
    );

    println!("\n--- overhead added by sync ---");
    println!("layouts, unchanged:   +{:.3} ms", l_on - l_off);
    println!("layouts, one edit:    +{:.3} ms", l_edit - l_off);
    println!("watchlists, unchanged:+{:.3} ms", w_on - w_off);
    println!(
        "alert heartbeat:      +{:.3} ms per 100 alerts",
        a_on - a_off
    );

    println!("\n--- applying a fetched batch ---");
    let applier = store(true);
    let records: Vec<RemoteRecord> = (1..=200u64)
        .map(|id| {
            let value = json!({"id": id, "name": format!("List {id}"), "entries": [], "display": {},
                "is_favorites": false, "created_at_ms": 1, "updated_at_ms": 2});
            let payload = serde_json::to_vec(&value).unwrap();
            RemoteRecord {
                record: SyncRecord {
                    name: RecordName::new(super::policy::EntityKind::Watchlist, id.to_string())
                        .unwrap(),
                    schema_version: 1,
                    revision: 1,
                    origin_device: "other".into(),
                    modified_ms: 1,
                    payload,
                },
                value,
                system_fields: vec![1],
            }
        })
        .collect();
    let batch = RemoteBatch {
        id: 1,
        records,
        deletions: vec![],
    };
    let ctx = ApplyCtx {
        device_id: "me".into(),
        device_name: "Mac".into(),
        now_ms: 1,
        date: "2026-10-20".into(),
    };
    let start = Instant::now();
    apply_batch(&applier, &mut NoScripts, &batch, &ctx).unwrap();
    println!(
        "apply 200 new watchlists in one transaction                      {:>9.3} ms",
        start.elapsed().as_secs_f64() * 1_000.0
    );
    let again = Instant::now();
    apply_batch(&applier, &mut NoScripts, &batch, &ctx).unwrap();
    println!(
        "re-apply the same 200 (idempotent)                               {:>9.3} ms",
        again.elapsed().as_secs_f64() * 1_000.0
    );

    println!("\n--- reconcile ---");
    let big = store(true);
    big.replace_documents("saved_view", &layouts).unwrap();
    big.replace_documents("watchlist", &lists).unwrap();
    timed(
        "reconcile 30 layouts (600 drawings) + 20 watchlists",
        20,
        || big.reconcile_stored().unwrap(),
    );
}
