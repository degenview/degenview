use super::alert::*;
use super::*;
use crate::model::alert::{
    AlertAsset, AlertPrecondition, AlertRuntimeCommand, AlertRuntimeCommandPayload,
};
use std::collections::HashMap;

/// What the app does with a request: plan against the snapshot as it is now, queue the save
/// with its precondition, and leave applying to the runtime (nothing is drained here).
fn queue_create(store: &Store, host: &AlertHost, request: &NewAlert) -> Option<u64> {
    let plan = plan_create_alert(
        host,
        &WatchlistQuoteBook::default(),
        &HashMap::new(),
        request,
        NOW,
    )
    .ok()?;
    let CreatePlan::New { alert, baseline } = plan else {
        return None;
    };
    let command =
        AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Save { alert, baseline }, NOW)
            .with_precondition(AlertPrecondition::Create {
                max_alerts: MAX_ALERTS,
            });
    store.enqueue_alert_command(&command).unwrap();
    Some(command.id)
}

fn queue_update(store: &Store, host: &AlertHost, request: &AlertUpdate) -> Option<u64> {
    let plan = plan_update_alert(
        host,
        &WatchlistQuoteBook::default(),
        &HashMap::new(),
        request,
        NOW,
    )
    .ok()?;
    let UpdatePlan::Change {
        alert,
        base,
        baseline,
    } = plan
    else {
        return None;
    };
    let command =
        AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Save { alert, baseline }, NOW)
            .with_precondition(AlertPrecondition::Unchanged(base));
    store.enqueue_alert_command(&command).unwrap();
    Some(command.id)
}

#[test]
fn identical_creates_in_one_frame_make_one_alert_and_the_other_is_refused() {
    let (store, mut host) = alert_host();
    let first = queue_create(&store, &host, &request(true, "70000")).unwrap();
    let second = queue_create(&store, &host, &request(true, "70000")).unwrap();
    assert_eq!(
        alert_command_outcome(&host, first),
        AlertCommandOutcome::Pending
    );
    host.drain_commands(&store, NOW);
    assert_eq!(host.snapshot().alerts.len(), 1);
    assert_eq!(
        alert_command_outcome(&host, first),
        AlertCommandOutcome::Applied
    );
    assert_eq!(
        alert_command_outcome(&host, second),
        AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Duplicate)
    );
}

#[test]
fn a_burst_of_creates_at_the_cap_cannot_exceed_it() {
    let (store, mut host) = alert_host();
    for index in 0..MAX_ALERTS - 1 {
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::CrossesAbove {
                target: 1.0 + index as f64,
            },
            NOW,
        );
        store
            .enqueue_alert_command(&AlertRuntimeCommand::new(
                AlertRuntimeCommandPayload::Save {
                    alert: Box::new(alert),
                    baseline: None,
                },
                NOW,
            ))
            .unwrap();
    }
    host.drain_commands(&store, NOW);
    assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS - 1);
    // Every one of these sees 499 alerts and passes the planner's own cap check.
    let ids: Vec<_> = ["900001", "900002", "900003"]
        .iter()
        .map(|target| queue_create(&store, &host, &request(true, target)).unwrap())
        .collect();
    host.drain_commands(&store, NOW);
    assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS);
    assert_eq!(
        alert_command_outcome(&host, ids[0]),
        AlertCommandOutcome::Applied
    );
    for id in &ids[1..] {
        assert_eq!(
            alert_command_outcome(&host, *id),
            AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::LimitReached)
        );
    }
}

#[test]
fn two_edits_of_different_settings_in_one_frame_do_not_lose_the_first() {
    let (store, mut host, id) = seeded();
    let mut note = update(id);
    note.note = Some("first".into());
    let mut frequency = update(id);
    frequency.frequency = Some(AlertFrequency::EveryTime);
    let one = queue_update(&store, &host, &note).unwrap();
    let two = queue_update(&store, &host, &frequency).unwrap();
    host.drain_commands(&store, NOW);
    let stored = &host.snapshot().alerts[0];
    assert_eq!(stored.note, "first", "the first edit is kept");
    assert_eq!(
        stored.frequency,
        AlertFrequency::Once,
        "not overwritten by a stale copy"
    );
    assert_eq!(
        alert_command_outcome(&host, one),
        AlertCommandOutcome::Applied
    );
    assert_eq!(
        alert_command_outcome(&host, two),
        AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Changed)
    );
}

#[test]
fn an_edit_and_a_delete_in_one_frame_agree_in_either_order() {
    // Delete first, then an edit planned from the old snapshot: the alert stays deleted.
    let (store, mut host, id) = seeded();
    let mut note = update(id);
    note.note = Some("late".into());
    let delete = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Delete(id), NOW);
    store.enqueue_alert_command(&delete).unwrap();
    let edit = queue_update(&store, &host, &note).unwrap();
    host.drain_commands(&store, NOW);
    assert!(host.snapshot().alerts.is_empty());
    assert_eq!(
        alert_command_outcome(&host, edit),
        AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Missing)
    );

    // Edit first, then the delete: the delete still wins.
    let (store, mut host, id) = seeded();
    let mut note = update(id);
    note.note = Some("early".into());
    let edit = queue_update(&store, &host, &note).unwrap();
    store
        .enqueue_alert_command(&AlertRuntimeCommand::new(
            AlertRuntimeCommandPayload::Delete(id),
            NOW,
        ))
        .unwrap();
    host.drain_commands(&store, NOW);
    assert!(host.snapshot().alerts.is_empty());
    assert_eq!(
        alert_command_outcome(&host, edit),
        AlertCommandOutcome::Applied
    );
}

#[test]
fn a_rejection_survives_a_reload_from_the_store() {
    // The background agent may apply commands; the app only sees them after a reload.
    let (store, mut host) = alert_host();
    let first = queue_create(&store, &host, &request(true, "70000")).unwrap();
    let second = queue_create(&store, &host, &request(true, "70000")).unwrap();
    host.drain_commands(&store, NOW);
    host.save_if_dirty(&store);
    let reloaded = AlertHost::load(&store, NOW);
    assert_eq!(
        alert_command_outcome(&reloaded, first),
        AlertCommandOutcome::Applied
    );
    assert!(matches!(
        alert_command_outcome(&reloaded, second),
        AlertCommandOutcome::Rejected(_)
    ));
}
