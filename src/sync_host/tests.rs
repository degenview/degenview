//! The host end to end: real worker threads, the real core and apply layer, a mock cloud.

use super::*;
use crate::store::Store;
use crate::sync::mock::{MockCloud, MockTransport};
use crate::sync::transport::TransportCommand;
use serde_json::{Value, json};

/// A transport that delivers the mock's events as soon as a command produces them, the way the
/// native bridge's callback would.
struct AutoTransport {
    inner: MockTransport,
    sink: EventSink,
}

impl SyncTransport for AutoTransport {
    fn send(&mut self, command: TransportCommand) {
        self.inner.send(command);
        for event in self.inner.take_events() {
            (self.sink)(event);
        }
    }
}

struct MockFactory {
    cloud: MockCloud,
    device: String,
    environment: Result<String, String>,
}

impl TransportFactory for MockFactory {
    fn environment(&self) -> Result<String, String> {
        self.environment.clone()
    }
    fn create(&self, events: EventSink) -> Box<dyn SyncTransport + Send> {
        Box::new(AutoTransport {
            inner: MockTransport::new(&self.cloud, &self.device, "acct"),
            sink: events,
        })
    }
}

struct Rig {
    host: SyncHost,
    store: SharedStore,
    scripts_dir: PathBuf,
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "degenview-synchost-{tag}-{}-{}",
        std::process::id(),
        crate::store::sync::new_device_id()
    ))
}

fn rig(cloud: &MockCloud, device: &str) -> Rig {
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let root = scratch(device);
    let scripts = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    let factory: Arc<dyn TransportFactory> = Arc::new(MockFactory {
        cloud: cloud.clone(),
        device: device.to_string(),
        environment: Ok("Development".into()),
    });
    let host = SyncHost::new(
        store.clone(),
        Some(factory),
        Some(scripts),
        root.join("Backups"),
        Arc::new(|| {}),
    );
    Rig {
        host,
        store,
        scripts_dir: root,
    }
}

impl Rig {
    /// Ticks the app side until `condition` holds, or fails after a few seconds.
    fn wait(&self, what: &str, condition: impl Fn(&SyncView) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.host.tick();
            let view = self.host.view();
            if condition(&view) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; phase {:?}, prompt {:?}, error {:?}",
                view.phase,
                view.prompt,
                view.last_error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn rows(&self, table: &str) -> Vec<Value> {
        self.store.lock().unwrap().raw_rows(table).unwrap()
    }

    fn put_watchlist(&self, id: u64, name: &str) {
        let list = json!({"id": id, "name": name, "entries": [], "display": {}, "is_favorites": false,
            "created_at_ms": 1, "updated_at_ms": 2});
        assert!(crate::store::sync::row_decodes(
            "watchlist",
            &list.to_string()
        ));
        self.store
            .lock()
            .unwrap()
            .replace_documents("watchlist", &[crate::store::sync::RawDoc(list)])
            .unwrap();
    }

    /// Enables sync, agrees to the account, and finishes the first-time merge.
    fn set_up(&self, mode: JoinMode) {
        self.host.act(SyncAction::Enable);
        self.wait("the account prompt", |v| {
            v.prompt == Some(SyncPrompt::ConfirmAccount)
        });
        self.host.act(SyncAction::ConfirmAccount);
        self.wait("the join prompt", |v| {
            matches!(v.prompt, Some(SyncPrompt::Join(_)))
        });
        self.host.act(SyncAction::ChooseJoin(mode));
        self.wait("up to date", |v| {
            v.phase == Phase::UpToDate && v.pending == 0
        });
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.host.shutdown();
        let _ = std::fs::remove_dir_all(&self.scripts_dir);
    }
}

#[test]
fn a_build_without_icloud_says_so_and_does_nothing() {
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let host = SyncHost::new(store.clone(), None, None, scratch("none"), Arc::new(|| {}));
    assert!(host.view().unavailable.is_some());
    host.act(SyncAction::Enable);
    host.tick();
    assert!(!store.lock().unwrap().sync_enabled());
    assert_eq!(host.view().phase, Phase::Off);
}

#[test]
fn an_unsigned_build_reports_why_icloud_cannot_work() {
    let cloud = MockCloud::new();
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let factory: Arc<dyn TransportFactory> = Arc::new(MockFactory {
        cloud,
        device: "A".into(),
        environment: Err("This copy of DegenView is not signed for iCloud.".into()),
    });
    let host = SyncHost::new(
        store.clone(),
        Some(factory),
        None,
        scratch("unsigned"),
        Arc::new(|| {}),
    );
    assert_eq!(
        host.view().unavailable.as_deref(),
        Some("This copy of DegenView is not signed for iCloud.")
    );
    host.act(SyncAction::Enable);
    host.tick();
    assert!(
        !store.lock().unwrap().sync_enabled(),
        "never enabled without the entitlement"
    );
}

#[test]
fn turning_sync_on_asks_before_using_the_account_and_before_merging() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.put_watchlist(10, "Mine");
    a.host.act(SyncAction::Enable);
    a.wait("the account prompt", |v| {
        v.prompt == Some(SyncPrompt::ConfirmAccount)
    });
    assert!(
        cloud.names().is_empty(),
        "nothing is read or sent before the user agrees"
    );
    a.host.act(SyncAction::ConfirmAccount);
    a.wait("the join prompt", |v| {
        matches!(v.prompt, Some(SyncPrompt::Join(_)))
    });
    assert!(
        cloud.names().is_empty(),
        "nothing is sent before the merge is chosen"
    );
    let Some(SyncPrompt::Join(summary)) = a.host.view().prompt else {
        unreachable!()
    };
    assert!(summary.cloud_is_empty());
    assert_eq!(summary.local.watchlists, 1);
    a.host.act(SyncAction::ChooseJoin(JoinMode::Merge));
    a.wait("up to date", |v| {
        v.phase == Phase::UpToDate && v.pending == 0
    });
    assert_eq!(cloud.names(), ["watchlist.10"]);
    assert_eq!(a.host.view().environment.as_deref(), Some("Development"));
}

#[test]
fn a_second_mac_receives_the_first_macs_data_through_the_app_thread() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.put_watchlist(10, "From A");
    a.set_up(JoinMode::Merge);

    let b = rig(&cloud, "Mac B");
    b.put_watchlist(20, "From B");
    b.host.act(SyncAction::Enable);
    b.wait("the account prompt", |v| {
        v.prompt == Some(SyncPrompt::ConfirmAccount)
    });
    b.host.act(SyncAction::ConfirmAccount);
    b.wait("the join prompt", |v| {
        matches!(v.prompt, Some(SyncPrompt::Join(_)))
    });
    let Some(SyncPrompt::Join(summary)) = b.host.view().prompt else {
        unreachable!()
    };
    assert_eq!(summary.cloud.watchlists, 1);
    b.host.act(SyncAction::ChooseJoin(JoinMode::Merge));
    b.wait("B up to date", |v| {
        v.phase == Phase::UpToDate && v.pending == 0
    });

    // A hears about B's list.
    a.host.act(SyncAction::SyncNow);
    a.wait("A has both lists", |_| a.rows("watchlist").len() == 2);
    assert_eq!(b.rows("watchlist").len(), 2);
}

#[test]
fn applying_reports_what_changed_so_the_app_can_reload_what_it_holds() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.set_up(JoinMode::Merge);
    let b = rig(&cloud, "Mac B");
    b.set_up(JoinMode::Merge);
    a.put_watchlist(30, "Later");
    a.host.act(SyncAction::SyncNow);
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut tables = std::collections::BTreeSet::new();
    b.host.act(SyncAction::SyncNow);
    while tables.is_empty() && Instant::now() < deadline {
        a.host.tick();
        for report in b.host.tick() {
            tables.extend(report.tables);
        }
        b.host.act(SyncAction::SyncNow);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(tables.contains("watchlist"), "{tables:?}");
}

#[test]
fn turning_sync_off_keeps_local_data_and_stops_the_worker() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.put_watchlist(10, "Mine");
    a.set_up(JoinMode::Merge);
    a.host.act(SyncAction::Disable);
    a.wait("off", |v| !v.enabled && v.phase == Phase::Off);
    assert_eq!(a.rows("watchlist").len(), 1);
    assert_eq!(
        cloud.names(),
        ["watchlist.10"],
        "nothing was deleted from iCloud"
    );
    a.put_watchlist(11, "While off");
    for _ in 0..20 {
        a.host.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        cloud.names(),
        ["watchlist.10"],
        "edits made while off are not sent"
    );
}

#[test]
fn a_category_switch_is_remembered_and_shown() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.set_up(JoinMode::Merge);
    a.host
        .act(SyncAction::SetCategory(Category::Scripts, false));
    a.wait("the switch", |v| {
        v.categories.contains(&(Category::Scripts, false))
    });
    assert!(
        a.host
            .view()
            .categories
            .contains(&(Category::Watchlists, true))
    );
}

#[test]
fn large_deletes_wait_for_confirmation_and_can_be_kept_in_icloud() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    let lists: Vec<_> = (1..=6u64)
        .map(|id| {
            crate::store::sync::RawDoc(json!({"id": id, "name": "x", "entries": [], "display": {},
                "is_favorites": false, "created_at_ms": 1, "updated_at_ms": 2}))
        })
        .collect();
    a.store
        .lock()
        .unwrap()
        .replace_documents("watchlist", &lists)
        .unwrap();
    a.set_up(JoinMode::Merge);
    assert_eq!(cloud.names().len(), 6);
    a.store
        .lock()
        .unwrap()
        .replace_documents::<crate::store::sync::RawDoc>("watchlist", &[])
        .unwrap();
    a.wait("held deletes", |v| v.held == 6);
    assert_eq!(cloud.names().len(), 6, "nothing was deleted yet");
    a.host.act(SyncAction::KeepHeldDeletes);
    a.wait("none held", |v| v.held == 0);
    assert_eq!(cloud.names().len(), 6, "kept in iCloud");

    let b = rig(&cloud, "Mac B");
    b.store
        .lock()
        .unwrap()
        .replace_documents("watchlist", &lists[..1])
        .unwrap();
    b.set_up(JoinMode::Merge);
    assert_eq!(b.rows("watchlist").len(), 6, "the other Mac still has them");
}

#[test]
fn deleting_icloud_data_unbinds_after_icloud_confirms_and_keeps_local_data() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.put_watchlist(10, "Mine");
    a.set_up(JoinMode::Merge);
    assert_eq!(cloud.names(), ["watchlist.10"]);
    a.host.act(SyncAction::DeleteCloudData);
    a.wait("sync off", |v| !v.enabled);
    assert!(cloud.names().is_empty(), "iCloud's copy is gone");
    assert_eq!(a.rows("watchlist").len(), 1, "this Mac's copy is kept");
    let state = a.store.lock().unwrap().sync_state().unwrap().unwrap();
    assert!(
        state.account_hash.is_none(),
        "forgotten, so turning sync on starts afresh"
    );
    assert!(!state.initial_sync_done);
}

#[test]
fn another_macs_deletion_offers_to_upload_again_and_does_so() {
    let cloud = MockCloud::new();
    let a = rig(&cloud, "Mac A");
    a.put_watchlist(10, "Mine");
    a.set_up(JoinMode::Merge);
    // Another Mac wipes iCloud; this Mac is told.
    cloud.delete_zone_contents();
    a.store
        .lock()
        .unwrap()
        .execute_raw("UPDATE sync_state SET enabled = enabled")
        .unwrap();
    a.host.tell(WorkerMsg::Event(TransportEvent::Zone(
        crate::sync::transport::ZoneEvent::Deleted,
    )));
    a.wait("the prompt", |v| {
        v.prompt == Some(SyncPrompt::RemoteDataDeleted)
    });
    assert_eq!(a.rows("watchlist").len(), 1);
    a.host
        .act(SyncAction::ResolveRemoteDeletion { reupload: true });
    a.wait("the join prompt", |v| {
        matches!(v.prompt, Some(SyncPrompt::Join(_)))
    });
    a.host.act(SyncAction::ChooseJoin(JoinMode::Merge));
    a.wait("up to date", |v| {
        v.phase == Phase::UpToDate && v.pending == 0
    });
    assert_eq!(
        cloud.names(),
        ["watchlist.10"],
        "uploaded again from this Mac"
    );
}

#[test]
fn today_is_a_calendar_date() {
    let text = super::today();
    assert_eq!(text.len(), 10);
    assert_eq!(&text[4..5], "-");
    assert_eq!(&text[7..8], "-");
    let year: i32 = text[..4].parse().unwrap();
    assert!(year >= 2025);
}
