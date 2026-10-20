//! Crash recovery. Each test stops the app at one of the points where a half-finished sync is
//! most dangerous, starts a fresh core on the same database (all that survives a crash), and
//! checks nothing was lost, duplicated or sent twice.

use super::apply::{ApplyCtx, apply_batch};
use super::apply_tests::{outbox_len, put, rows, watchlist};
use super::core::{Phase, SyncCore};
use super::core_tests::{Mac, mac};
use super::mock::{MockCloud, MockTransport};
use super::scripts::NoScripts;

fn pair() -> (MockCloud, Mac, Mac) {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "Mac A", "acct", Some("acct"));
    let mut b = mac(&cloud, "Mac B", "acct", Some("acct"));
    a.apply = true;
    b.apply = true;
    (cloud, a, b)
}

/// The app was killed and relaunched: a new core on the same database, a new connection.
fn restart(mac: &mut Mac, cloud: &MockCloud, name: &str) {
    let now = mac.now.clone();
    let mut core = SyncCore::new(
        mac.store.clone(),
        MockTransport::new(cloud, name, "acct"),
        Box::new(move || now.load(std::sync::atomic::Ordering::Relaxed)),
    );
    core.set_script_cache(mac.script_cache.clone());
    mac.core = core;
    mac.core.start();
    mac.run();
}

fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..5 {
        a.core.request_sync();
        a.run();
        b.core.request_sync();
        b.run();
    }
}

#[test]
fn a_change_made_just_before_a_crash_uploads_after_restart() {
    let (cloud, mut a, _b) = pair();
    // Saved and queued in one transaction, then the app dies before any upload.
    put(&a, &[watchlist(10, "Just drawn", &[(1, "AAA")], false)]);
    assert_eq!(outbox_len(&a), 1);
    restart(&mut a, &cloud, "Mac A");
    assert_eq!(cloud.names(), ["watchlist.10"]);
    assert_eq!(outbox_len(&a), 0);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn a_crash_after_the_cloud_accepted_but_before_the_acknowledgement_does_not_duplicate() {
    let (cloud, mut a, mut b) = pair();
    put(&a, &[watchlist(10, "Once", &[(1, "AAA")], false)]);
    a.core.start();
    // Deliver the account event only: this stages and sends. The cloud accepts the record.
    let events = a.core.transport_mut().take_events();
    for event in events {
        a.deliver(event);
    }
    assert_eq!(cloud.save_attempts(), 1);
    // The acknowledgement is in the transport queue, but the process dies before reading it.
    assert_eq!(outbox_len(&a), 1);
    restart(&mut a, &cloud, "Mac A");
    b.core.start();
    assert_eq!(cloud.names(), ["watchlist.10"], "one logical record");
    assert_eq!(outbox_len(&a), 0);
    assert_eq!(a.phase(), Phase::UpToDate);
    settle(&mut a, &mut b);
    assert_eq!(
        rows(&b, "watchlist").len(),
        1,
        "and one list on the other Mac"
    );
}

#[test]
fn a_crash_during_a_batch_upload_resumes_without_losing_or_repeating_records() {
    let (cloud, mut a, mut b) = pair();
    let rows_to_make: Vec<_> = (1..=6).map(|id| (id, "x")).collect();
    let lists: Vec<_> = rows_to_make
        .iter()
        .map(|(id, name)| watchlist(*id + 100, name, &[], false))
        .collect();
    put(&a, &lists);
    a.core.start();
    let events = a.core.transport_mut().take_events();
    for event in events {
        a.deliver(event);
    }
    restart(&mut a, &cloud, "Mac A");
    b.core.start();
    settle(&mut a, &mut b);
    assert_eq!(cloud.names().len(), 6);
    assert_eq!(rows(&b, "watchlist").len(), 6);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_failed_apply_rolls_back_and_the_changes_arrive_again_after_restart() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.run();
    b.run();
    put(&a, &[watchlist(10, "From A", &[(1, "AAA")], false)]);
    a.core.request_sync();
    a.run();

    // B fetches, but its disk fails while applying, and the app dies.
    b.hold_applies = true;
    b.core.request_sync();
    b.run();
    let batch = b.batches.last().cloned().expect("B received the change");
    b.store
        .lock()
        .unwrap()
        .execute_raw(
            "CREATE TRIGGER fail_apply BEFORE INSERT ON watchlist BEGIN SELECT RAISE(ABORT, 'disk full'); END",
        )
        .unwrap();
    let state = b.store.lock().unwrap().sync_state().unwrap().unwrap();
    let ctx = ApplyCtx {
        device_id: state.device_id.clone(),
        device_name: state.device_name,
        now_ms: 1,
        date: "2026-10-20".into(),
    };
    {
        let store = b.store.lock().unwrap();
        assert!(apply_batch(&store, &mut NoScripts, &batch, &ctx).is_err());
    }
    assert!(rows(&b, "watchlist").is_empty(), "nothing half-applied");
    let checkpoint = b
        .store
        .lock()
        .unwrap()
        .sync_state()
        .unwrap()
        .unwrap()
        .engine_state;

    // Relaunch with the disk healthy. The checkpoint never moved past the failed batch.
    b.store
        .lock()
        .unwrap()
        .execute_raw("DROP TRIGGER fail_apply")
        .unwrap();
    b.hold_applies = false;
    restart(&mut b, &cloud, "Mac B");
    assert_eq!(
        b.store
            .lock()
            .unwrap()
            .sync_state()
            .unwrap()
            .unwrap()
            .engine_state
            .is_some(),
        checkpoint.is_some()
    );
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "watchlist").len(), 1);
    assert_eq!(outbox_len(&b), 0);
}

#[test]
fn a_crash_after_applying_but_before_saving_the_checkpoint_reapplies_harmlessly() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.run();
    b.run();
    put(&a, &[watchlist(10, "From A", &[(1, "AAA")], false)]);
    a.core.request_sync();
    a.run();

    // B applies the change, then dies before the core records that it did.
    b.hold_applies = true;
    b.core.request_sync();
    b.run();
    let batch = b.batches.last().cloned().unwrap();
    b.apply_to_store(&batch);
    let after_first = rows(&b, "watchlist");
    assert_eq!(after_first.len(), 1);

    b.hold_applies = false;
    restart(&mut b, &cloud, "Mac B");
    settle(&mut a, &mut b);
    assert_eq!(
        rows(&b, "watchlist"),
        after_first,
        "applying twice changed nothing"
    );
    assert_eq!(outbox_len(&b), 0, "and nothing was echoed back");
    assert_eq!(cloud.save_attempts(), 1);
}

#[test]
fn a_crash_between_two_edits_of_the_same_item_keeps_the_latest() {
    let (cloud, mut a, _b) = pair();
    put(&a, &[watchlist(10, "v1", &[], false)]);
    a.core.start();
    let events = a.core.transport_mut().take_events();
    for event in events {
        a.deliver(event);
    }
    put(&a, &[watchlist(10, "v2", &[], false)]);
    restart(&mut a, &cloud, "Mac A");
    let record = cloud.record("watchlist.10").unwrap();
    let value: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(value["name"], "v2");
    assert_eq!(outbox_len(&a), 0);
}

#[test]
fn a_crash_while_signed_out_keeps_the_queue_for_when_the_account_returns() {
    let (cloud, mut a, _b) = pair();
    a.core.start();
    a.run();
    a.core.transport_mut().sign_out();
    a.run();
    put(&a, &[watchlist(10, "Offline", &[], false)]);
    restart(&mut a, &cloud, "Mac A");
    assert_eq!(
        outbox_len(&a),
        0,
        "the new launch is signed in, so it uploads"
    );
    assert_eq!(cloud.names(), ["watchlist.10"]);
}

#[test]
fn duplicate_and_out_of_order_deliveries_do_not_duplicate_anything() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.run();
    b.run();
    put(&a, &[watchlist(10, "v1", &[(1, "AAA")], false)]);
    a.core.request_sync();
    a.run();
    b.core.request_sync();
    b.run();
    let first = b.batches.last().cloned().unwrap();
    put(&a, &[watchlist(10, "v2", &[(1, "AAA")], false)]);
    a.core.request_sync();
    a.run();
    b.core.request_sync();
    b.run();
    // The older delivery shows up again, and the newer one twice.
    b.apply_to_store(&first);
    let latest = b.batches.last().cloned().unwrap();
    b.apply_to_store(&latest);
    b.apply_to_store(&latest);
    assert_eq!(rows(&b, "watchlist").len(), 1);
    assert_eq!(rows(&b, "watchlist")[0]["name"], "v2");
    assert_eq!(outbox_len(&b), 0);
}
