//! The first-time merge and account changes, end to end with the real core, apply layer and a
//! mock cloud.

use super::account::{AccountChoice, resolve_account_change};
use super::apply::ApplyCtx;
use super::apply_tests::{outbox_len, put, rows, symbols, template, watchlist};
use super::core::{Effect, PauseReason, Phase, SyncCore};
use super::core_tests::{Mac, mac_with};
use super::initial::{JoinMode, JoinSummary, finish_join, summarize};
use super::mock::{MockCloud, MockTransport};
use std::path::PathBuf;

const V1: &str = "//@version=6\nindicator(\"V1\")\nplot(close)\n";
const V2: &str = "//@version=6\nindicator(\"V2\")\nplot(open)\n";

fn new_mac(cloud: &MockCloud, name: &str) -> Mac {
    let mut mac = mac_with(cloud, name, "acct", Some("acct"), false);
    mac.apply = true;
    mac
}

fn backups(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "degenview-sync-join-{tag}-{}-{}",
        std::process::id(),
        crate::store::sync::new_device_id()
    ))
}

fn summary(mac: &Mac) -> JoinSummary {
    let store = mac.store.lock().unwrap();
    summarize(&store, mac.library.scripts.len()).unwrap()
}

fn join(mac: &mut Mac, mode: JoinMode, dir: &std::path::Path) -> super::initial::JoinOutcome {
    let state = mac.store.lock().unwrap().sync_state().unwrap().unwrap();
    let ctx = ApplyCtx {
        device_id: state.device_id,
        device_name: state.device_name,
        now_ms: 20_000_000,
        date: "2026-10-20".into(),
    };
    let outcome = {
        let store = mac.store.lock().unwrap();
        finish_join(&store, &mut mac.library, &mac.script_cache, mode, dir, &ctx).unwrap()
    };
    mac.run();
    outcome
}

fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..6 {
        a.scan_scripts();
        a.core.request_sync();
        a.run();
        b.scan_scripts();
        b.core.request_sync();
        b.run();
    }
}

/// Both Macs exchange everything, past the paper upload debounce.
fn settle_papers(a: &mut Mac, b: &mut Mac) {
    a.advance(6_000);
    b.advance(6_000);
    settle(a, b);
}

/// Lets one Mac's changes reach iCloud, past the paper upload debounce.
fn settle_pair(a: &mut Mac, _cloud: &MockCloud) {
    a.advance(6_000);
    for _ in 0..4 {
        a.core.request_sync();
        a.run();
    }
}

/// A Mac that has used iCloud for a while and holds some data.
fn established(cloud: &MockCloud, name: &str) -> Mac {
    let mut mac = new_mac(cloud, name);
    put(&mac, &[watchlist(10, "Shared", &[(1, "AAA")], false)]);
    mac.library.add("Cloud Script", V1);
    mac.store
        .lock()
        .unwrap()
        .replace_documents("chart_template", &[template(1500, "Cloud Look")])
        .unwrap();
    mac.core.start();
    mac.run();
    let dir = backups("established");
    join(&mut mac, JoinMode::Merge, &dir);
    mac
}

#[test]
fn the_first_mac_uploads_its_data_after_seeing_that_icloud_is_empty() {
    let cloud = MockCloud::new();
    let mut a = new_mac(&cloud, "A");
    put(&a, &[watchlist(10, "Mine", &[(1, "AAA")], false)]);
    a.library.add("My Script", V1);
    a.core.start();
    a.run();
    assert_eq!(a.phase(), Phase::AwaitingJoin);
    assert!(
        cloud.names().is_empty(),
        "nothing uploads before the choice"
    );
    let seen = summary(&a);
    assert!(seen.cloud_is_empty());
    assert_eq!(seen.local.watchlists, 1);
    assert_eq!(seen.local.scripts, 1);

    join(&mut a, JoinMode::Merge, &backups("first"));
    assert!(cloud.names().contains(&"watchlist.10".to_string()));
    assert_eq!(
        cloud
            .names()
            .iter()
            .filter(|n| n.starts_with("script."))
            .count(),
        1
    );
    assert_eq!(a.phase(), Phase::UpToDate);
    assert_eq!(outbox_len(&a), 0);
    assert_eq!(
        rows(&a, "watchlist").len(),
        1,
        "local data is never removed"
    );
}

#[test]
fn an_empty_icloud_never_deletes_local_data() {
    let cloud = MockCloud::new();
    let mut a = new_mac(&cloud, "A");
    put(
        &a,
        &[
            watchlist(10, "Mine", &[], false),
            watchlist(11, "Also mine", &[], false),
        ],
    );
    a.core.start();
    a.run();
    join(&mut a, JoinMode::Merge, &backups("empty"));
    settle(&mut a, &mut new_mac(&cloud, "B"));
    assert_eq!(rows(&a, "watchlist").len(), 2);
}

#[test]
fn a_second_mac_joins_existing_data_and_neither_side_loses_anything() {
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    // B has used the app for months and has its own things.
    put(&b, &[watchlist(20, "B's list", &[(2, "BBB")], false)]);
    b.library.add("B Script", V2);
    b.core.start();
    b.run();

    assert_eq!(b.phase(), Phase::AwaitingJoin);
    let seen = summary(&b);
    assert_eq!(seen.cloud.watchlists, 1);
    assert_eq!(seen.cloud.scripts, 1);
    assert_eq!(seen.cloud.templates, 1);
    assert_eq!(seen.local.watchlists, 1);
    assert_eq!(seen.cloud_devices.len(), 1);
    let uploaded_before = cloud.names();
    assert!(
        rows(&b, "chart_template").is_empty(),
        "nothing is applied before the choice"
    );
    assert_eq!(cloud.names(), uploaded_before, "and nothing is uploaded");

    join(&mut b, JoinMode::Merge, &backups("second"));
    settle(&mut a, &mut b);

    for mac in [&a, &b] {
        let names: Vec<String> = rows(mac, "watchlist")
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect();
        assert!(
            names.contains(&"Shared".to_string()) && names.contains(&"B's list".to_string()),
            "{names:?}"
        );
        assert_eq!(rows(mac, "chart_template").len(), 1);
        let scripts: Vec<String> = mac.library.sorted().into_iter().map(|(n, _)| n).collect();
        assert_eq!(scripts, ["B Script", "Cloud Script"]);
    }
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
    assert_eq!(b.phase(), Phase::UpToDate);
}

/// Paper accounts a Mac holds, by name.
fn paper_names(mac: &Mac) -> Vec<String> {
    let mut names: Vec<String> = mac
        .store
        .lock()
        .unwrap()
        .paper_trading()
        .unwrap()
        .accounts
        .into_iter()
        .map(|account| account.name)
        .collect();
    names.sort();
    names
}

fn paper_host(mac: &Mac) -> crate::host::paper_host::PaperHost {
    crate::host::paper_host::PaperHost::load(mac.store.clone())
}

#[test]
fn an_untouched_default_paper_account_gives_way_to_the_one_in_icloud_on_join() {
    use crate::model::paper::{PaperAccountSettings, PaperCurrency};
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    paper_host(&a)
        .create_account(
            "Main",
            PaperCurrency::Usd,
            50_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    settle_pair(&mut a, &cloud);

    // B opened the paper panel before joining, which made its default account.
    let mut b = new_mac(&cloud, "B");
    paper_host(&b).ensure_default_account().unwrap();
    assert_eq!(paper_names(&b), ["Paper Trading"]);
    b.core.start();
    b.run();
    join(&mut b, JoinMode::Merge, &backups("paper-default"));
    settle_papers(&mut a, &mut b);

    for mac in [&a, &b] {
        assert_eq!(paper_names(mac), ["Main"], "no second \"Paper Trading\"");
    }
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_paper_account_that_was_used_is_kept_on_join() {
    use crate::model::paper::{PaperAccountSettings, PaperCurrency};
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    paper_host(&a)
        .create_account(
            "Main",
            PaperCurrency::Usd,
            50_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    settle_pair(&mut a, &cloud);

    let mut b = new_mac(&cloud, "B");
    let mut on_b = paper_host(&b);
    let id = on_b.ensure_default_account().unwrap().expect("default");
    // A resting order is something the user did.
    on_b.submit(super::paper_tests::limit_buy(id, 1.0)).unwrap();
    b.core.start();
    b.run();
    join(&mut b, JoinMode::Merge, &backups("paper-used"));
    settle_papers(&mut a, &mut b);

    for mac in [&a, &b] {
        assert_eq!(paper_names(mac), ["Main", "Paper Trading"]);
    }
}

#[test]
fn the_default_paper_account_stays_when_icloud_has_no_paper_accounts() {
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    paper_host(&b).ensure_default_account().unwrap();
    b.core.start();
    b.run();
    join(&mut b, JoinMode::Merge, &backups("paper-none"));
    settle_papers(&mut a, &mut b);
    for mac in [&a, &b] {
        assert_eq!(paper_names(mac), ["Paper Trading"]);
    }
}

#[test]
fn the_same_list_on_both_macs_merges_into_one() {
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    // B independently has the same list id with a different market (as if it had synced before).
    put(&b, &[watchlist(10, "Shared", &[(9, "ZZZ")], false)]);
    b.core.start();
    b.run();
    join(&mut b, JoinMode::Merge, &backups("same"));
    settle(&mut a, &mut b);
    let on_a = rows(&a, "watchlist");
    assert_eq!(on_a.len(), 1);
    let mut markets = symbols(&on_a[0]);
    markets.sort();
    assert_eq!(markets, ["AAA", "ZZZ"]);
    assert_eq!(rows(&a, "watchlist"), rows(&b, "watchlist"));
}

#[test]
fn using_icloud_stashes_a_recoverable_copy_and_deletes_nothing_in_the_cloud() {
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    put(&b, &[watchlist(20, "B's list", &[(2, "BBB")], false)]);
    b.library.add("B Script", V2);
    b.core.start();
    b.run();
    let cloud_before = cloud.names();
    let dir = backups("use");

    let outcome = join(&mut b, JoinMode::UseCloud, &dir);
    settle(&mut a, &mut b);

    let backup = outcome.backup.expect("a copy was stashed");
    let restored = crate::store::Store::open(&backup).unwrap();
    assert_eq!(
        restored.raw_rows("watchlist").unwrap()[0]["name"],
        "B's list"
    );
    // B now shows iCloud's lists, not its old ones.
    let names: Vec<String> = rows(&b, "watchlist")
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Shared"]);
    // Nothing was deleted from iCloud, and B's old list was not uploaded.
    for name in &cloud_before {
        assert!(cloud.names().contains(name), "{name} is still in iCloud");
    }
    assert!(!cloud.names().contains(&"watchlist.20".to_string()));
    // Scripts are merged, never replaced.
    let scripts: Vec<String> = b.library.sorted().into_iter().map(|(n, _)| n).collect();
    assert_eq!(scripts, ["B Script", "Cloud Script"]);
    assert_eq!(rows(&a, "watchlist").len(), 1);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keeping_local_only_turns_sync_off_and_changes_nothing() {
    let cloud = MockCloud::new();
    let a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    put(&b, &[watchlist(20, "B's list", &[], false)]);
    b.core.start();
    b.run();
    let before = cloud.names();
    join(&mut b, JoinMode::KeepLocalOnly, &backups("keep"));
    assert!(!b.store.lock().unwrap().sync_enabled());
    assert!(b.store.lock().unwrap().inbox().unwrap().is_empty());
    assert_eq!(rows(&b, "watchlist").len(), 1);
    assert_eq!(cloud.names(), before);
    assert_eq!(rows(&a, "watchlist").len(), 1);
}

#[test]
fn running_the_join_twice_after_an_interruption_changes_nothing_more() {
    let cloud = MockCloud::new();
    let mut a = established(&cloud, "A");
    let mut b = new_mac(&cloud, "B");
    put(&b, &[watchlist(20, "B's list", &[], false)]);
    b.core.start();
    b.run();
    let dir = backups("twice");
    join(&mut b, JoinMode::Merge, &dir);
    settle(&mut a, &mut b);
    let (la, lb) = (rows(&a, "watchlist"), rows(&b, "watchlist"));
    let saves = cloud.save_attempts();
    // The app was killed before it noticed it had finished; the join runs again.
    {
        let mut state = b.store.lock().unwrap().sync_state().unwrap().unwrap();
        state.initial_sync_done = false;
        b.store.lock().unwrap().save_sync_state(&state).unwrap();
    }
    join(&mut b, JoinMode::Merge, &dir);
    settle(&mut a, &mut b);
    assert_eq!(rows(&a, "watchlist"), la);
    assert_eq!(rows(&b, "watchlist"), lb);
    assert_eq!(cloud.save_attempts(), saves, "nothing was uploaded twice");
}

#[test]
fn alerts_arriving_in_the_join_are_not_deleted_before_the_alert_owner_runs() {
    use crate::model::alert::{
        AlertAsset, AlertCondition, AlertRuntimeCommand, AlertRuntimeCommandPayload, PriceAlert,
    };
    use crate::model::{DataSource, InstrumentId};
    let cloud = MockCloud::new();
    let mut a = new_mac(&cloud, "A");
    a.core.start();
    a.run();
    join(&mut a, JoinMode::Merge, &backups("alerts-a"));
    for target in [100.0, 200.0] {
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "btcusdt"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::CrossesAbove { target },
            1,
        );
        let command = AlertRuntimeCommand::new(
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: Some(1.0),
            },
            1,
        );
        a.store
            .lock()
            .unwrap()
            .enqueue_alert_command(&command)
            .unwrap();
    }
    {
        let store = a.store.lock().unwrap();
        let mut engine = crate::model::alert_engine::AlertEngine::new(None);
        let commands = store.pending_alert_commands();
        for c in &commands {
            engine.apply(c, 1);
        }
        let ids: Vec<u64> = commands.iter().map(|c| c.id).collect();
        store
            .save_alert_snapshot_and_acknowledge(engine.snapshot(), &ids)
            .unwrap();
    }
    a.run();
    let alerts_in_cloud = cloud
        .names()
        .iter()
        .filter(|n| n.starts_with("alert."))
        .count();
    assert_eq!(alerts_in_cloud, 2);

    let mut b = new_mac(&cloud, "B");
    b.core.start();
    b.run();
    join(&mut b, JoinMode::Merge, &backups("alerts-b"));
    // The commands are queued but B's alert owner has not run yet: nothing is queued to delete.
    assert!(
        b.store
            .lock()
            .unwrap()
            .outbox()
            .unwrap()
            .iter()
            .all(|row| row.op != crate::store::sync::OutboxOp::Delete)
    );
    settle(&mut a, &mut b);
    assert_eq!(
        cloud
            .names()
            .iter()
            .filter(|n| n.starts_with("alert."))
            .count(),
        2
    );
}

// MARK: Account changes

#[test]
fn a_different_account_never_receives_the_old_accounts_data_without_a_choice() {
    let cloud_one = MockCloud::new();
    let cloud_two = MockCloud::new();
    let mut a = established(&cloud_one, "A");
    put(
        &a,
        &[
            watchlist(10, "Shared", &[(1, "AAA")], false),
            watchlist(30, "Private", &[], false),
        ],
    );
    a.run();
    assert!(cloud_one.names().contains(&"watchlist.30".to_string()));

    // The user signs into another Apple Account; the app restarts its engine for it.
    let mut core = SyncCore::new(
        a.store.clone(),
        MockTransport::new(&cloud_two, "A", "account-two"),
        Box::new(|| 30_000_000),
    );
    core.start();
    let mut effects = Vec::new();
    for _ in 0..5 {
        for event in core.transport_mut().take_events() {
            effects.extend(core.handle(event));
        }
        core.tick();
    }
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::AccountMismatch { .. }))
    );
    assert_eq!(core.phase(), Phase::Paused(PauseReason::AccountChanged));
    assert!(
        cloud_two.names().is_empty(),
        "nothing crossed to the new account"
    );
    assert_eq!(
        rows(&a, "watchlist").len(),
        2,
        "and nothing local was removed"
    );
}

#[test]
fn merging_into_the_new_account_uploads_this_macs_data_only_after_the_choice() {
    let cloud_one = MockCloud::new();
    let cloud_two = MockCloud::new();
    let a = established(&cloud_one, "A");
    let account_hash = "account-two";

    let dir = backups("switch");
    {
        let store = a.store.lock().unwrap();
        let resolution = resolve_account_change(
            &store,
            AccountChoice::MergeIntoThisAccount,
            account_hash,
            &dir,
            40_000_000,
        )
        .unwrap();
        assert!(resolution.needs_join);
    }
    let mut core = SyncCore::new(
        a.store.clone(),
        MockTransport::new(&cloud_two, "A", account_hash),
        Box::new(|| 40_000_000),
    );
    core.start();
    for _ in 0..5 {
        for event in core.transport_mut().take_events() {
            core.handle(event);
        }
        core.tick();
    }
    assert_eq!(core.phase(), Phase::AwaitingJoin);
    assert!(
        cloud_two.names().is_empty(),
        "still nothing until the join runs"
    );

    let state = a.store.lock().unwrap().sync_state().unwrap().unwrap();
    let ctx = ApplyCtx {
        device_id: state.device_id,
        device_name: "A".into(),
        now_ms: 40_000_001,
        date: "2026-10-20".into(),
    };
    let mut library = super::core_tests::FakeLibrary::default();
    let cache = super::scripts::ScriptCache::default();
    {
        let store = a.store.lock().unwrap();
        finish_join(&store, &mut library, &cache, JoinMode::Merge, &dir, &ctx).unwrap();
    }
    for _ in 0..5 {
        for event in core.transport_mut().take_events() {
            core.handle(event);
        }
        core.tick();
    }
    assert!(cloud_two.names().contains(&"watchlist.10".to_string()));
    assert_eq!(core.phase(), Phase::UpToDate);
}
