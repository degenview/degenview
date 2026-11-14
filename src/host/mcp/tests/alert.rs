use super::*;

use crate::model::alert::{AlertAsset, AlertRuntimeCommand, AlertRuntimeCommandPayload};
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use std::collections::HashMap;

pub(super) const NOW: i64 = 1_760_000_000_000;

pub(super) fn alert_host() -> (Store, AlertHost) {
    let store = Store::open_in_memory().unwrap();
    let host = AlertHost::load(&store, NOW);
    (store, host)
}

pub(super) fn command(store: &Store, host: &mut AlertHost, payload: AlertRuntimeCommandPayload) {
    store
        .enqueue_alert_command(&AlertRuntimeCommand::new(payload, NOW))
        .unwrap();
    host.drain_commands(store, NOW);
}

pub(super) fn request(crosses_above: bool, target: &str) -> NewAlert {
    NewAlert {
        instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
        crosses_above,
        target_text: target.to_owned(),
        currency: AlertCurrency::Usd,
        frequency: AlertFrequency::Once,
        note: String::new(),
    }
}

pub(super) fn book_with_price(last: f64) -> WatchlistQuoteBook {
    let book = WatchlistQuoteBook::default();
    book.apply(HashMap::from([(
        InstrumentId::new(DataSource::Binance, "BTCUSDT").key(),
        WatchlistQuote {
            last: Some(last),
            freshness: Freshness::Live,
            ..WatchlistQuote::default()
        },
    )]));
    book
}

/// Creates through the same two steps the app takes: plan, then queue the Save.
pub(super) fn create(
    store: &Store,
    host: &mut AlertHost,
    book: &WatchlistQuoteBook,
    request: &NewAlert,
) -> Result<CreatePlan, McpError> {
    let plan = plan_create_alert(host, book, &HashMap::new(), request, NOW)?;
    if let CreatePlan::New { alert, baseline } = &plan {
        command(
            store,
            host,
            AlertRuntimeCommandPayload::Save {
                alert: alert.clone(),
                baseline: *baseline,
            },
        );
    }
    Ok(plan)
}

#[test]
fn a_created_alert_is_built_by_the_sheets_rules_and_armed_from_the_live_price() {
    let (store, mut host) = alert_host();
    // Price below the target: the next cross upward counts.
    let below = book_with_price(65_000.0);
    let plan = create(&store, &mut host, &below, &request(true, "70000")).unwrap();
    let CreatePlan::New { alert, baseline } = plan else {
        panic!("expected a new alert");
    };
    assert_eq!(baseline, Some(65_000.0));
    assert_eq!(
        alert.condition,
        AlertCondition::CrossesAbove { target: 70_000.0 }
    );
    assert_eq!(alert.state, AlertState::Active);
    assert_eq!(alert.currency, AlertCurrency::Usd);
    assert_eq!(alert.frequency, AlertFrequency::Once);
    assert_eq!(alert.asset.key(), "BINANCE:BTCUSDT");
    assert!(alert.webhook_endpoint_ids.is_empty());
    assert!(host.snapshot().alerts[0].armed);

    // Price already above the target: it must first fall back and cross again.
    let above = book_with_price(80_000.0);
    create(&store, &mut host, &above, &request(true, "75000")).unwrap();
    let second = host
        .snapshot()
        .alerts
        .iter()
        .find(|a| a.condition.target() == Some(75_000.0))
        .unwrap();
    assert!(!second.armed);

    // No live price: still created, and armed by default.
    let none = WatchlistQuoteBook::default();
    let plan = create(&store, &mut host, &none, &request(false, "50000")).unwrap();
    assert!(matches!(plan, CreatePlan::New { baseline: None, .. }));
}

#[test]
fn listing_reports_the_alert_and_never_exposes_webhooks() {
    let (store, mut host) = alert_host();
    let mut alert = PriceAlert::new(
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        ),
        AlertCondition::CrossesBelow { target: 0.00000278 },
        NOW,
    );
    alert.note = "dip".into();
    alert.webhook_endpoint_ids = vec![424_242];
    let id = alert.id;
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(alert),
            baseline: None,
        },
    );
    let listed = list_alerts(&host, None);
    assert_eq!((listed.total, listed.truncated), (1, false));
    let info = &listed.alerts[0];
    assert_eq!(info.alert_id, id.to_string());
    assert_eq!(info.condition, AlertConditionKind::CrossesBelow);
    assert_eq!(info.target_price.as_deref(), Some("0.00000278"));
    assert_eq!(
        (info.status, info.frequency),
        (AlertStatus::Active, AlertFrequencyKind::Once)
    );
    assert_eq!(info.note, "dip");
    assert!(info.sends_to_webhooks);
    let json = serde_json::to_string(&listed).unwrap();
    assert!(
        !json.contains("424242") && !json.contains("webhook_endpoint"),
        "{json}"
    );
    assert!(info.created_at.ends_with('Z'));

    assert_eq!(list_alerts(&host, Some(id)).alerts.len(), 1);
    assert!(list_alerts(&host, Some(id + 1)).alerts.is_empty());
}

#[test]
fn an_identical_alert_is_returned_instead_of_a_duplicate() {
    let (store, mut host) = alert_host();
    let book = WatchlistQuoteBook::default();
    create(&store, &mut host, &book, &request(true, "70000")).unwrap();
    let id = host.snapshot().alerts[0].id.to_string();

    let again = create(&store, &mut host, &book, &request(true, "70000")).unwrap();
    assert!(matches!(&again, CreatePlan::Existing(info) if info.alert_id == id));
    assert_eq!(host.snapshot().alerts.len(), 1);

    // A different level, direction, currency or frequency is a different alert.
    for different in [
        request(true, "71000"),
        request(false, "70000"),
        NewAlert {
            currency: AlertCurrency::Eur,
            ..request(true, "70000")
        },
        NewAlert {
            frequency: AlertFrequency::EveryTime,
            ..request(true, "70000")
        },
    ] {
        let plan = create(&store, &mut host, &book, &different).unwrap();
        assert!(matches!(plan, CreatePlan::New { .. }));
    }
    assert_eq!(host.snapshot().alerts.len(), 5);
}

#[test]
fn an_unparseable_target_is_refused() {
    let (_store, host) = alert_host();
    let book = WatchlistQuoteBook::default();
    for bad in ["", "0", "-1", "abc"] {
        let result = plan_create_alert(&host, &book, &HashMap::new(), &request(true, bad), NOW);
        assert!(
            matches!(result, Err(McpError::InvalidArgument(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn the_alert_count_is_capped() {
    let (store, mut host) = alert_host();
    for index in 0..MAX_ALERTS {
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
    assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS);
    let result = plan_create_alert(
        &host,
        &WatchlistQuoteBook::default(),
        &HashMap::new(),
        &request(true, "999999"),
        NOW,
    );
    assert!(matches!(result, Err(McpError::ResourceLimit(_))));
    assert_eq!(list_alerts(&host, None).alerts.len(), ALERT_PAGE);
    assert!(list_alerts(&host, None).truncated);
}

#[test]
fn pause_resume_and_delete_follow_the_alert_windows_semantics() {
    let (store, mut host) = alert_host();
    create(
        &store,
        &mut host,
        &WatchlistQuoteBook::default(),
        &request(true, "70000"),
    )
    .unwrap();
    let id = host.snapshot().alerts[0].id;

    // Active: pausing changes it, resuming is a no-op.
    assert!(matches!(
        plan_pause_alert(&host, id, false),
        Ok(StatePlan::Already)
    ));
    let Ok(StatePlan::Change {
        id: change_id,
        state,
    }) = plan_pause_alert(&host, id, true)
    else {
        panic!("expected a change");
    };
    assert_eq!((change_id, state), (id, AlertState::Paused));
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::SetState {
            id,
            state,
            baseline: None,
        },
    );
    assert_eq!(
        list_alerts(&host, Some(id)).alerts[0].status,
        AlertStatus::Paused
    );
    assert!(matches!(
        plan_pause_alert(&host, id, true),
        Ok(StatePlan::Already)
    ));

    // A fired once-alert is resumed (re-armed) like the Toggle button does.
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::SetState {
            id,
            state: AlertState::Triggered,
            baseline: None,
        },
    );
    assert!(matches!(
        plan_pause_alert(&host, id, false),
        Ok(StatePlan::Change {
            state: AlertState::Active,
            ..
        })
    ));

    // Delete.
    assert_eq!(plan_delete_alert(&host, id).unwrap(), id);
    command(&store, &mut host, AlertRuntimeCommandPayload::Delete(id));
    assert!(list_alerts(&host, None).alerts.is_empty());
    assert!(matches!(
        plan_delete_alert(&host, id),
        Err(McpError::NotFound(_))
    ));
    assert!(matches!(
        plan_pause_alert(&host, id, true),
        Err(McpError::NotFound(_))
    ));
}

#[test]
fn an_alert_with_a_condition_from_a_newer_version_cannot_be_toggled() {
    let (store, mut host) = alert_host();
    let alert = PriceAlert::new(
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        ),
        AlertCondition::Unsupported {
            tag: "touchesVwap".into(),
        },
        NOW,
    );
    let id = alert.id;
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(alert),
            baseline: None,
        },
    );
    assert!(matches!(
        plan_pause_alert(&host, id, true),
        Err(McpError::Unsupported(_))
    ));
    let info = &list_alerts(&host, Some(id)).alerts[0];
    assert_eq!(
        (info.condition, info.status),
        (AlertConditionKind::Unsupported, AlertStatus::Unsupported)
    );
    assert_eq!(info.target_price, None);
}

#[test]
fn decimals_drop_float_noise_but_keep_real_digits() {
    assert_eq!(decimal(70_000.0), "70000");
    assert_eq!(decimal(110.000_000_000_000_01), "110");
    assert_eq!(decimal(0.000_002_78), "0.00000278");
    assert_eq!(decimal(61_234.567_89), "61234.56789");
    assert_eq!(decimal(2.5), "2.5");
    assert_eq!(decimal(1e-8), "0.00000001");
}

#[test]
fn percent_conditions_report_their_reference() {
    let alert = PriceAlert::new(
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        ),
        AlertCondition::rises_by(10.0, 100.0),
        NOW,
    );
    let info = alert_info(&alert, Some(NOW));
    assert_eq!(info.condition, AlertConditionKind::RisesBy);
    assert_eq!(info.percent.as_deref(), Some("10"));
    assert_eq!(info.reference_price.as_deref(), Some("100"));
    assert_eq!(info.target_price.as_deref(), Some("110"));
    assert!(info.last_triggered_at.is_some());
}

pub(super) fn update(id: AlertId) -> AlertUpdate {
    AlertUpdate {
        alert_id: id,
        crosses_above: None,
        target_text: None,
        currency: None,
        frequency: None,
        note: None,
    }
}

pub(super) fn apply_update(
    store: &Store,
    host: &mut AlertHost,
    book: &WatchlistQuoteBook,
    request: &AlertUpdate,
) -> Result<UpdatePlan, McpError> {
    let plan = plan_update_alert(host, book, &HashMap::new(), request, NOW)?;
    if let UpdatePlan::Change {
        alert, baseline, ..
    } = &plan
    {
        command(
            store,
            host,
            AlertRuntimeCommandPayload::Save {
                alert: alert.clone(),
                baseline: *baseline,
            },
        );
    }
    Ok(plan)
}

pub(super) fn seeded() -> (Store, AlertHost, AlertId) {
    let (store, mut host) = alert_host();
    create(
        &store,
        &mut host,
        &WatchlistQuoteBook::default(),
        &request(true, "70000"),
    )
    .unwrap();
    let id = host.snapshot().alerts[0].id;
    (store, host, id)
}

#[test]
fn editing_changes_only_the_given_settings_and_keeps_the_alert() {
    let (store, mut host, id) = seeded();
    let book = WatchlistQuoteBook::default();
    let created_at = host.snapshot().alerts[0].created_at_ms;

    let mut edit = update(id);
    edit.target_text = Some("72500.5".into());
    edit.note = Some("breakout".into());
    assert!(matches!(
        apply_update(&store, &mut host, &book, &edit),
        Ok(UpdatePlan::Change { .. })
    ));
    let alert = &host.snapshot().alerts[0];
    assert_eq!(host.snapshot().alerts.len(), 1);
    assert_eq!(alert.id, id);
    assert_eq!(alert.created_at_ms, created_at);
    assert_eq!(
        alert.condition,
        AlertCondition::CrossesAbove { target: 72_500.5 }
    );
    assert_eq!(alert.note, "breakout");
    assert_eq!(alert.frequency, AlertFrequency::Once);

    let mut edit = update(id);
    edit.crosses_above = Some(false);
    edit.currency = Some(AlertCurrency::Eur);
    edit.frequency = Some(AlertFrequency::EveryTime);
    apply_update(&store, &mut host, &book, &edit).unwrap();
    let alert = &host.snapshot().alerts[0];
    assert_eq!(
        alert.condition,
        AlertCondition::CrossesBelow { target: 72_500.5 }
    );
    assert_eq!(
        (alert.currency, alert.frequency),
        (AlertCurrency::Eur, AlertFrequency::EveryTime)
    );
    assert_eq!(alert.note, "breakout");
}

#[test]
fn an_edit_that_changes_nothing_queues_nothing() {
    let (_store, host, id) = seeded();
    let book = WatchlistQuoteBook::default();
    let mut same = update(id);
    same.target_text = Some("70000".into());
    same.crosses_above = Some(true);
    same.frequency = Some(AlertFrequency::Once);
    same.note = Some(String::new());
    assert!(matches!(
        plan_update_alert(&host, &book, &HashMap::new(), &same, NOW),
        Ok(UpdatePlan::Unchanged(_))
    ));
}

#[test]
fn an_edit_is_rearmed_from_the_live_price_like_the_sheet() {
    let (store, mut host, id) = seeded();
    // Move the target below a price of 80,000: it must fall back and cross up again.
    let mut edit = update(id);
    edit.target_text = Some("75000".into());
    apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
    assert!(!host.snapshot().alerts[0].armed);
    let mut edit = update(id);
    edit.target_text = Some("90000".into());
    apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
    assert!(host.snapshot().alerts[0].armed);
}

#[test]
fn a_remote_edit_never_resumes_a_paused_alert() {
    let (store, mut host, id) = seeded();
    let book = WatchlistQuoteBook::default();
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::SetState {
            id,
            state: AlertState::Paused,
            baseline: None,
        },
    );
    let mut edit = update(id);
    edit.target_text = Some("71000".into());
    apply_update(&store, &mut host, &book, &edit).unwrap();
    assert_eq!(host.snapshot().alerts[0].state, AlertState::Paused);
    assert_eq!(host.snapshot().alerts[0].condition.target(), Some(71_000.0));
}

#[test]
fn a_fired_alert_is_rearmed_only_by_a_real_change() {
    let (store, mut host, id) = seeded();
    let book = WatchlistQuoteBook::default();
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::SetState {
            id,
            state: AlertState::Triggered,
            baseline: None,
        },
    );
    // A note alone leaves it fired.
    let mut edit = update(id);
    edit.note = Some("seen".into());
    apply_update(&store, &mut host, &book, &edit).unwrap();
    assert_eq!(host.snapshot().alerts[0].state, AlertState::Triggered);
    // A new level makes it live again.
    let mut edit = update(id);
    edit.target_text = Some("99000".into());
    apply_update(&store, &mut host, &book, &edit).unwrap();
    assert_eq!(host.snapshot().alerts[0].state, AlertState::Active);
}

#[test]
fn an_edit_cannot_duplicate_another_alert() {
    let (store, mut host, id) = seeded();
    let book = WatchlistQuoteBook::default();
    create(&store, &mut host, &book, &request(true, "80000")).unwrap();
    let mut edit = update(id);
    edit.target_text = Some("80000".into());
    let result = plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW);
    assert!(matches!(result, Err(McpError::Conflict(_))));
    assert_eq!(
        host.snapshot()
            .alerts
            .iter()
            .filter(|a| a.condition.target() == Some(80_000.0))
            .count(),
        1
    );
}

#[test]
fn edits_keep_webhooks_and_refuse_what_cannot_be_edited() {
    let (store, mut host) = alert_host();
    let book = WatchlistQuoteBook::default();
    let asset = AlertAsset::new(
        InstrumentId::new(DataSource::Binance, "BTCUSDT"),
        "BTC",
        "Bitcoin",
    );

    let mut hooked = PriceAlert::new(
        asset.clone(),
        AlertCondition::CrossesAbove { target: 70_000.0 },
        NOW,
    );
    hooked.webhook_endpoint_ids = vec![7];
    hooked.webhook_message = Some("{{price}}".into());
    let hooked_id = hooked.id;
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(hooked),
            baseline: None,
        },
    );
    let mut edit = update(hooked_id);
    edit.target_text = Some("71000".into());
    apply_update(&store, &mut host, &book, &edit).unwrap();
    let alert = host
        .snapshot()
        .alerts
        .iter()
        .find(|a| a.id == hooked_id)
        .unwrap();
    assert_eq!(alert.webhook_endpoint_ids, vec![7]);
    assert_eq!(alert.webhook_message.as_deref(), Some("{{price}}"));

    // A percentage alert takes a note and frequency, not a new level.
    let percent = PriceAlert::new(asset.clone(), AlertCondition::rises_by(10.0, 100.0), NOW);
    let percent_id = percent.id;
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(percent),
            baseline: None,
        },
    );
    let mut level = update(percent_id);
    level.target_text = Some("150".into());
    assert!(matches!(
        plan_update_alert(&host, &book, &HashMap::new(), &level, NOW),
        Err(McpError::Unsupported(_))
    ));
    let mut note = update(percent_id);
    note.note = Some("watch".into());
    note.frequency = Some(AlertFrequency::EveryTime);
    apply_update(&store, &mut host, &book, &note).unwrap();
    let alert = host
        .snapshot()
        .alerts
        .iter()
        .find(|a| a.id == percent_id)
        .unwrap();
    assert!(
        matches!(alert.condition, AlertCondition::RisesBy { percent, reference, .. } if percent == 10.0 && reference == 100.0)
    );
    assert_eq!(
        (alert.note.as_str(), alert.frequency),
        ("watch", AlertFrequency::EveryTime)
    );

    // A condition from a newer version, and an alert that does not exist.
    let future = PriceAlert::new(asset, AlertCondition::Unsupported { tag: "x".into() }, NOW);
    let future_id = future.id;
    command(
        &store,
        &mut host,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(future),
            baseline: None,
        },
    );
    let mut edit = update(future_id);
    edit.note = Some("x".into());
    assert!(matches!(
        plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW),
        Err(McpError::Unsupported(_))
    ));
    assert!(matches!(
        plan_update_alert(&host, &book, &HashMap::new(), &update(1), NOW),
        Err(McpError::NotFound(_))
    ));
    let mut bad = update(hooked_id);
    bad.target_text = Some("0".into());
    assert!(matches!(
        plan_update_alert(&host, &book, &HashMap::new(), &bad, NOW),
        Err(McpError::InvalidArgument(_))
    ));
}
